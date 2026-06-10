import base64, json, os, subprocess, sys
from pathlib import Path

CALLSIEVE = "/Users/pjb/Git/callsieve/target/release/callsieve"
tasks = {}
for line in open("/tmp/gapstudy/tasks.tsv"):
    issue_id, b64 = line.rstrip("\n").split("\t")
    tasks[issue_id] = base64.b64decode(b64).decode()

results = []
for line in open("/tmp/gapstudy/misses.tsv"):
    issue_id, repo, commit, truth = line.rstrip("\n").split("\t")
    repo_dir = Path("/tmp/gapstudy") / repo.split("/")[0]
    subprocess.run(["git", "-C", str(repo_dir), "checkout", "-q", "-f", commit], check=True)
    callsieve_dir = repo_dir / ".callsieve"
    if callsieve_dir.exists():
        subprocess.run(["rm", "-rf", str(callsieve_dir)], check=True)
    subprocess.run([CALLSIEVE, "index", str(repo_dir)], capture_output=True, check=True)
    out = subprocess.run(
        [CALLSIEVE, "agent-context", str(repo_dir), tasks[issue_id], "--limit", "8", "--no-daemon"],
        capture_output=True, check=True)
    packet = json.loads(out.stdout)
    pool = [f["f"] for f in packet["context"]["read_first"]]

    index = json.loads((callsieve_dir / "index.json").read_text())
    pool_set = set(pool)
    truth_dir = os.path.dirname(truth)

    # adjacency: imports (resolved_path) and references (target_path)
    pool_imports_truth, truth_imports_pool = set(), set()
    for imp in index.get("imports", []):
        src, dst = imp.get("source_path"), imp.get("resolved_path")
        if not dst: continue
        if src in pool_set and dst == truth: pool_imports_truth.add(src)
        if src == truth and dst in pool_set: truth_imports_pool.add(dst)
    pool_refs_truth, truth_refs_pool = set(), set()
    for ref in index.get("references", []):
        src, dst = ref.get("source_path"), ref.get("target_path")
        if not dst: continue
        if src in pool_set and dst == truth: pool_refs_truth.add(src)
        if src == truth and dst in pool_set: truth_refs_pool.add(dst)
    same_dir = [f for f in pool if os.path.dirname(f) == truth_dir]

    reachable = bool(pool_imports_truth or truth_imports_pool or pool_refs_truth or truth_refs_pool or same_dir)
    results.append({
        "id": issue_id, "truth": truth, "reachable": reachable,
        "pool_imports_truth": sorted(pool_imports_truth), "truth_imports_pool": sorted(truth_imports_pool),
        "pool_refs_truth": sorted(pool_refs_truth), "truth_refs_pool": sorted(truth_refs_pool),
        "same_dir": same_dir, "pool": pool,
    })
    flags = "".join([
        "I" if pool_imports_truth else "-", "i" if truth_imports_pool else "-",
        "R" if pool_refs_truth else "-", "r" if truth_refs_pool else "-",
        "D" if same_dir else "-"])
    print(f"{issue_id}: {'REACHABLE' if reachable else 'isolated '} [{flags}] {truth}", flush=True)

json.dump(results, open("/tmp/gapstudy/results.json", "w"), indent=1)
n = len(results); r = sum(1 for x in results if x["reachable"])
print(f"\n1-hop reachable: {r}/{n} ({100*r/n:.0f}%)")
for key, label in [("pool_imports_truth","pool imports truth"), ("truth_imports_pool","truth imports pool"),
                   ("pool_refs_truth","pool refs truth"), ("truth_refs_pool","truth refs pool"), ("same_dir","same dir")]:
    c = sum(1 for x in results if x[key])
    print(f"  {label}: {c}/{n}")
