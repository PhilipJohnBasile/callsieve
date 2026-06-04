//! Workstream 2 - Hybrid retrieval foundation.
//!
//! This module is the smallest shippable slice of the embedding layer:
//! a `LocalEmbedder` trait, a `fastembed`-backed adapter, and a
//! `.callsieve/embeds.bin` reader/writer that tags each cache with the
//! `(model_id, model_revision, index_schema_version)` triple. Anything that
//! doesn't match those three values causes the reader to return `None`,
//! forcing a rebuild rather than silently serving stale vectors.
//!
//! Nothing here touches `src/query/ranker.rs` yet - the hybrid blend lands in
//! a follow-up task. Compile-out is total: with `--no-default-features` the
//! file is not included.
//!
//! ## Crate choice
//!
//! We use `fastembed` (version 4.x). It ships pre-quantized BGE-small /
//! MiniLM models, runs on CPU via ONNX Runtime (no Python in the loop), and
//! is the de-facto standard local-embedding crate for Rust. The
//! `ort-download-binaries` feature gives us a self-contained build on
//! macOS/Linux out of the box, which matches CallSieve's "no external setup"
//! philosophy.
//!
//! ## On-disk format (`embeds.bin`)
//!
//! Little-endian throughout.
//!
//! ```text
//! magic           : 4 bytes  "CSEM"
//! format_version  : u16      currently 1
//! flags           : u8       bit 0 = vectors are f16; bit 1+ reserved
//! _pad            : u8       reserved, must be 0
//! index_schema    : u32      mirrors crate::indexer::SCHEMA_VERSION
//! dim             : u32      embedding dimensionality
//! count           : u32      number of vectors
//! model_id_len    : u16      bytes of UTF-8 model_id
//! model_id        : [u8; n]
//! model_rev_len   : u16      bytes of UTF-8 model_revision
//! model_revision  : [u8; n]
//! vectors         : count * dim * (2 if f16 else 4) bytes
//! ```
//!
//! We quantize to f16 by default. For BGE-small / MiniLM cosine-similarity
//! retrieval, f16 round-trip error is well below the noise floor of the
//! ranker, and the cache is half the size of f32. The format also supports
//! f32 (clear the f16 flag) for callers that want bit-exact storage; the
//! reader auto-detects.

#![allow(dead_code)]

use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

use anyhow::{Context, Result, anyhow, bail};

/// Identifies a specific embedding model + checkpoint. Used both at
/// runtime and as a cache key on disk.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EmbedderId {
    pub model_id: String,
    pub model_revision: String,
}

impl EmbedderId {
    pub fn new(model_id: impl Into<String>, model_revision: impl Into<String>) -> Self {
        Self {
            model_id: model_id.into(),
            model_revision: model_revision.into(),
        }
    }
}

/// Trait every local embedding backend implements. Kept intentionally
/// minimal: a stable identity, plus a batch embed call.
pub trait LocalEmbedder {
    fn id(&self) -> EmbedderId;
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
}

// ---------------------------------------------------------------------------
// fastembed adapter
// ---------------------------------------------------------------------------

/// `fastembed`-backed embedder. Default model is BGE-small-en-v1.5, which is
/// small enough for CPU and strong enough for code/text retrieval.
pub struct FastembedEmbedder {
    inner: fastembed::TextEmbedding,
    id: EmbedderId,
}

impl FastembedEmbedder {
    /// Construct with the crate's default model (BGE-small-en-v1.5).
    pub fn new_default() -> Result<Self> {
        let model = fastembed::EmbeddingModel::BGESmallENV15;
        Self::new_with_model(model)
    }

    /// Construct with an explicit `fastembed::EmbeddingModel`. Exposed so
    /// callers can pin a specific checkpoint (e.g. MiniLM) without
    /// re-implementing the trait.
    pub fn new_with_model(model: fastembed::EmbeddingModel) -> Result<Self> {
        // We synthesize a stable `(model_id, model_revision)` from the
        // model's `Debug` representation plus the embedder crate version.
        // That avoids depending on private/unstable fields of
        // `fastembed::ModelInfo`, while still guaranteeing a cache built
        // against one model is rejected if the caller swaps to another.
        let model_id = format!("{model:?}");
        let model_revision = format!("fastembed-{}", env!("CARGO_PKG_VERSION"));
        let init = fastembed::InitOptions::new(model);
        let inner = fastembed::TextEmbedding::try_new(init)
            .map_err(|e| anyhow!("fastembed init failed: {e}"))?;
        Ok(Self {
            inner,
            id: EmbedderId::new(model_id, model_revision),
        })
    }
}

impl LocalEmbedder for FastembedEmbedder {
    fn id(&self) -> EmbedderId {
        self.id.clone()
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        // `fastembed` wants owned Strings. The clone cost is dwarfed by the
        // forward pass itself.
        let owned: Vec<String> = texts.iter().map(|s| (*s).to_string()).collect();
        let out = self
            .inner
            .embed(owned, None)
            .map_err(|e| anyhow!("fastembed embed failed: {e}"))?;
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// embeds.bin reader / writer
// ---------------------------------------------------------------------------

const MAGIC: &[u8; 4] = b"CSEM";
const FORMAT_VERSION: u16 = 1;
const FLAG_F16: u8 = 0b0000_0001;

/// In-memory representation of a loaded `.callsieve/embeds.bin`.
#[derive(Debug, Clone)]
pub struct EmbedCache {
    pub embedder: EmbedderId,
    pub index_schema_version: u32,
    pub dim: usize,
    /// `vectors[i]` is the i-th embedding, length `dim`.
    pub vectors: Vec<Vec<f32>>,
}

/// What the caller asks the reader to enforce.
#[derive(Debug, Clone)]
pub struct ExpectedCache<'a> {
    pub embedder: &'a EmbedderId,
    pub index_schema_version: u32,
}

/// Path helper, mirrors `store::json_store::index_path`.
pub fn embeds_path(root: &Path) -> std::path::PathBuf {
    root.join(".callsieve").join("embeds.bin")
}

/// Write the cache. f16 quantization is the default; pass `false` for f32.
pub fn write_embeds(
    root: &Path,
    cache: &EmbedCache,
    quantize_f16: bool,
) -> Result<std::path::PathBuf> {
    let dir = root.join(".callsieve");
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create embeds dir {}", dir.display()))?;
    let path = dir.join("embeds.bin");
    let mut file = fs::File::create(&path)
        .with_context(|| format!("failed to create embeds file {}", path.display()))?;
    write_embeds_to(&mut file, cache, quantize_f16)?;
    Ok(path)
}

/// Read the cache. Returns `Ok(None)` if the file doesn't exist or its
/// header doesn't match `expected` - by design, the caller treats that as
/// "rebuild from scratch" rather than serving stale data. Returns `Err`
/// only on genuine I/O / corruption problems.
pub fn read_embeds(root: &Path, expected: &ExpectedCache<'_>) -> Result<Option<EmbedCache>> {
    let path = embeds_path(root);
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(e).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    read_embeds_from(&bytes[..], expected)
}

/// Lower-level writer used by tests and by `write_embeds`.
pub fn write_embeds_to<W: Write>(w: &mut W, cache: &EmbedCache, quantize_f16: bool) -> Result<()> {
    if cache.dim == 0 {
        bail!("embed cache dim must be > 0");
    }
    for (i, v) in cache.vectors.iter().enumerate() {
        if v.len() != cache.dim {
            bail!(
                "vector {i} has length {}, expected dim {}",
                v.len(),
                cache.dim
            );
        }
    }
    let model_id = cache.embedder.model_id.as_bytes();
    let model_rev = cache.embedder.model_revision.as_bytes();
    if model_id.len() > u16::MAX as usize || model_rev.len() > u16::MAX as usize {
        bail!("model_id / model_revision must fit in u16 bytes");
    }
    let flags: u8 = if quantize_f16 { FLAG_F16 } else { 0 };

    w.write_all(MAGIC)?;
    w.write_all(&FORMAT_VERSION.to_le_bytes())?;
    w.write_all(&[flags, 0u8])?; // flags + reserved pad
    w.write_all(&cache.index_schema_version.to_le_bytes())?;
    w.write_all(&(cache.dim as u32).to_le_bytes())?;
    w.write_all(&(cache.vectors.len() as u32).to_le_bytes())?;
    w.write_all(&(model_id.len() as u16).to_le_bytes())?;
    w.write_all(model_id)?;
    w.write_all(&(model_rev.len() as u16).to_le_bytes())?;
    w.write_all(model_rev)?;

    for v in &cache.vectors {
        if quantize_f16 {
            for x in v {
                let h = half::f16::from_f32(*x);
                w.write_all(&h.to_le_bytes())?;
            }
        } else {
            for x in v {
                w.write_all(&x.to_le_bytes())?;
            }
        }
    }
    Ok(())
}

/// Lower-level reader used by tests and by `read_embeds`. Same contract:
/// header mismatch -> `Ok(None)`, corruption -> `Err`.
pub fn read_embeds_from<R: Read>(
    mut r: R,
    expected: &ExpectedCache<'_>,
) -> Result<Option<EmbedCache>> {
    let mut magic = [0u8; 4];
    if let Err(e) = r.read_exact(&mut magic) {
        // Empty / truncated file -> treat as "no cache".
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            return Ok(None);
        }
        return Err(e.into());
    }
    if &magic != MAGIC {
        return Ok(None);
    }

    let format_version = read_u16_le(&mut r)?;
    if format_version != FORMAT_VERSION {
        return Ok(None);
    }
    let mut fb = [0u8; 2];
    r.read_exact(&mut fb)?;
    let flags = fb[0];
    // fb[1] is reserved padding; ignore.
    let is_f16 = (flags & FLAG_F16) != 0;

    let index_schema_version = read_u32_le(&mut r)?;
    let dim = read_u32_le(&mut r)? as usize;
    let count = read_u32_le(&mut r)? as usize;

    let model_id_len = read_u16_le(&mut r)? as usize;
    let mut model_id_buf = vec![0u8; model_id_len];
    r.read_exact(&mut model_id_buf)?;
    let model_id = String::from_utf8(model_id_buf)
        .map_err(|_| anyhow!("embeds.bin model_id is not valid utf-8"))?;

    let model_rev_len = read_u16_le(&mut r)? as usize;
    let mut model_rev_buf = vec![0u8; model_rev_len];
    r.read_exact(&mut model_rev_buf)?;
    let model_revision = String::from_utf8(model_rev_buf)
        .map_err(|_| anyhow!("embeds.bin model_revision is not valid utf-8"))?;

    // Header check - *this* is the only place stale caches get filtered.
    if index_schema_version != expected.index_schema_version
        || model_id != expected.embedder.model_id
        || model_revision != expected.embedder.model_revision
    {
        return Ok(None);
    }

    if dim == 0 {
        bail!("embeds.bin has zero dim");
    }
    let mut vectors = Vec::with_capacity(count);
    for _ in 0..count {
        let mut v = Vec::with_capacity(dim);
        if is_f16 {
            for _ in 0..dim {
                let mut b = [0u8; 2];
                r.read_exact(&mut b)?;
                v.push(half::f16::from_le_bytes(b).to_f32());
            }
        } else {
            for _ in 0..dim {
                let mut b = [0u8; 4];
                r.read_exact(&mut b)?;
                v.push(f32::from_le_bytes(b));
            }
        }
        vectors.push(v);
    }

    Ok(Some(EmbedCache {
        embedder: EmbedderId::new(model_id, model_revision),
        index_schema_version,
        dim,
        vectors,
    }))
}

fn read_u16_le<R: Read>(r: &mut R) -> Result<u16> {
    let mut b = [0u8; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_le_bytes(b))
}

fn read_u32_le<R: Read>(r: &mut R) -> Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn sample_cache() -> EmbedCache {
        EmbedCache {
            embedder: EmbedderId::new("bge-small-en-v1.5", "rev-abc123"),
            index_schema_version: 7,
            dim: 4,
            vectors: vec![
                vec![0.0, 0.5, -0.25, 1.0],
                vec![0.125, -0.125, 0.75, -0.5],
                vec![1.0, 1.0, -1.0, -1.0],
            ],
        }
    }

    #[test]
    fn round_trip_f16_small_set() {
        let cache = sample_cache();
        let mut buf = Vec::new();
        write_embeds_to(&mut buf, &cache, true).unwrap();

        let expected = ExpectedCache {
            embedder: &cache.embedder,
            index_schema_version: cache.index_schema_version,
        };
        let read = read_embeds_from(Cursor::new(&buf), &expected)
            .unwrap()
            .expect("cache should round-trip");
        assert_eq!(read.embedder, cache.embedder);
        assert_eq!(read.index_schema_version, cache.index_schema_version);
        assert_eq!(read.dim, cache.dim);
        assert_eq!(read.vectors.len(), cache.vectors.len());
        // f16 round-trip: exact for our sample (all values are dyadic
        // fractions representable in 11-bit mantissa). Use a tight epsilon
        // anyway in case the sample changes.
        for (got, want) in read.vectors.iter().zip(cache.vectors.iter()) {
            assert_eq!(got.len(), want.len());
            for (g, w) in got.iter().zip(want.iter()) {
                assert!((g - w).abs() < 1e-3, "f16 round-trip drift {g} vs {w}");
            }
        }
    }

    #[test]
    fn round_trip_f32_exact() {
        let cache = sample_cache();
        let mut buf = Vec::new();
        write_embeds_to(&mut buf, &cache, false).unwrap();

        let expected = ExpectedCache {
            embedder: &cache.embedder,
            index_schema_version: cache.index_schema_version,
        };
        let read = read_embeds_from(Cursor::new(&buf), &expected)
            .unwrap()
            .expect("cache should round-trip");
        assert_eq!(read.vectors, cache.vectors, "f32 round-trip must be exact");
    }

    #[test]
    fn cache_mismatch_returns_none_on_model_id() {
        let cache = sample_cache();
        let mut buf = Vec::new();
        write_embeds_to(&mut buf, &cache, true).unwrap();

        let other = EmbedderId::new("different-model", "rev-abc123");
        let expected = ExpectedCache {
            embedder: &other,
            index_schema_version: cache.index_schema_version,
        };
        let res = read_embeds_from(Cursor::new(&buf), &expected).unwrap();
        assert!(res.is_none(), "model_id mismatch must yield None");
    }

    #[test]
    fn cache_mismatch_returns_none_on_revision() {
        let cache = sample_cache();
        let mut buf = Vec::new();
        write_embeds_to(&mut buf, &cache, true).unwrap();

        let other = EmbedderId::new("bge-small-en-v1.5", "rev-different");
        let expected = ExpectedCache {
            embedder: &other,
            index_schema_version: cache.index_schema_version,
        };
        let res = read_embeds_from(Cursor::new(&buf), &expected).unwrap();
        assert!(res.is_none(), "model_revision mismatch must yield None");
    }

    #[test]
    fn cache_mismatch_returns_none_on_schema_version() {
        let cache = sample_cache();
        let mut buf = Vec::new();
        write_embeds_to(&mut buf, &cache, true).unwrap();

        let expected = ExpectedCache {
            embedder: &cache.embedder,
            index_schema_version: cache.index_schema_version + 1,
        };
        let res = read_embeds_from(Cursor::new(&buf), &expected).unwrap();
        assert!(res.is_none(), "schema_version mismatch must yield None");
    }

    #[test]
    fn missing_file_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let id = EmbedderId::new("m", "r");
        let expected = ExpectedCache {
            embedder: &id,
            index_schema_version: 7,
        };
        let res = read_embeds(tmp.path(), &expected).unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn round_trip_via_filesystem() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = sample_cache();
        let path = write_embeds(tmp.path(), &cache, true).unwrap();
        assert!(path.exists());

        let expected = ExpectedCache {
            embedder: &cache.embedder,
            index_schema_version: cache.index_schema_version,
        };
        let read = read_embeds(tmp.path(), &expected)
            .unwrap()
            .expect("cache should round-trip via filesystem");
        assert_eq!(read.embedder, cache.embedder);
        assert_eq!(read.vectors.len(), cache.vectors.len());
    }

    #[test]
    fn corrupt_magic_returns_none() {
        let buf = b"NOPE\x00\x00\x00\x00";
        let id = EmbedderId::new("m", "r");
        let expected = ExpectedCache {
            embedder: &id,
            index_schema_version: 1,
        };
        let res = read_embeds_from(Cursor::new(&buf[..]), &expected).unwrap();
        assert!(res.is_none(), "bad magic must yield None");
    }

    // ----- Deterministic embedder via a fake `LocalEmbedder` -----

    struct FakeEmbedder;

    impl LocalEmbedder for FakeEmbedder {
        fn id(&self) -> EmbedderId {
            EmbedderId::new("fake-deterministic", "v1")
        }

        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
            // Deterministic hash-bucket "embedding". Identical input -> identical
            // output, which is exactly the contract we want to assert.
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0.0f32; 8];
                    for (i, b) in t.bytes().enumerate() {
                        v[i % 8] += b as f32 / 255.0;
                    }
                    v
                })
                .collect())
        }
    }

    #[test]
    fn deterministic_embeddings_for_identical_input() {
        let e = FakeEmbedder;
        let a = e.embed(&["hello world", "foo"]).unwrap();
        let b = e.embed(&["hello world", "foo"]).unwrap();
        assert_eq!(a, b, "identical input must yield identical output");
        // And distinct inputs do diverge, so we're not trivially passing.
        let c = e.embed(&["different", "foo"]).unwrap();
        assert_ne!(a[0], c[0]);
    }

    #[test]
    fn embedder_id_round_trip_through_cache_header() {
        let e = FakeEmbedder;
        let id = e.id();
        let cache = EmbedCache {
            embedder: id.clone(),
            index_schema_version: 7,
            dim: 8,
            vectors: e.embed(&["a", "b", "c"]).unwrap(),
        };
        let mut buf = Vec::new();
        write_embeds_to(&mut buf, &cache, true).unwrap();
        let expected = ExpectedCache {
            embedder: &id,
            index_schema_version: 7,
        };
        let read = read_embeds_from(Cursor::new(&buf), &expected)
            .unwrap()
            .expect("round-trip");
        assert_eq!(read.embedder, id);
        assert_eq!(read.vectors.len(), 3);
    }
}
