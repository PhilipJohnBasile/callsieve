import base64, json, subprocess, sys
from pathlib import Path

CALLSIEVE = "/Users/pjb/Git/callsieve/target/release/callsieve"
tasks = {}
for line in open("/tmp/gapstudy/tasks.tsv"):
    issue_id, b64 = line.rstrip("\n").split("\t")
    tasks[issue_id] = base64.b64decode(b64).decode()

hits = 0; total = 0
for line in open("/tmp/gapstudy/misses.tsv"):
    issue_id, repo, commit, truth = line.rstrip("\n").split("\t")
    repo_dir = Path("/tmp/gapstudy") / repo.split("/")[0]
    subprocess.run(["git", "-C", str(repo_dir), "checkout", "-q", "-f", commit], check=True)
    subprocess.run(["rm", "-rf", str(repo_dir / ".callsieve")], check=True)
    subprocess.run([CALLSIEVE, "index", str(repo_dir)], capture_output=True, check=True)
    out = subprocess.run(
        [CALLSIEVE, "agent-context", str(repo_dir), tasks[issue_id], "--limit", "8", "--no-daemon"],
        capture_output=True, check=True)
    pool = [f["f"] for f in json.loads(out.stdout)["context"]["read_first"]]
    top5 = pool[:5]
    total += 1
    hit = truth in top5
    hits += hit
    print(f"{issue_id}: {'HIT ' if hit else 'miss'} truth={truth} rank={pool.index(truth) if truth in pool else '>8'}", flush=True)
print(f"\nconverted: {hits}/{total}")
