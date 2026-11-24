"""Starts the first server and this one on copies of one database file."""
import os, shutil, subprocess, sys, time, urllib.request
HERE = os.path.dirname(os.path.abspath(__file__))
BINARY = os.environ["STELLARR_API"]
NAME = os.environ.get("ORACLE", "msgviews")
CONTAINER = "stellarr-port-oracle-" + NAME
ORACLE_PORT = os.environ.get("ORACLE_PORT", "5215")
RUST_PORT = os.environ.get("RUST_PORT", "5316")
ORACLE = "http://127.0.0.1:" + ORACLE_PORT
RUST = "http://127.0.0.1:" + RUST_PORT
_rust = None
def ready(base):
    for _ in range(90):
        try:
            with urllib.request.urlopen(base + "/health/ready", timeout=2) as r:
                if r.status == 200: return
        except Exception: pass
        time.sleep(1)
    raise SystemExit(base + " did not become ready")
def reset_oracle(db):
    subprocess.run(["docker", "stop", "-t", "10", CONTAINER], check=True, capture_output=True)
    subprocess.run(["docker", "cp", db, CONTAINER + ":/tmp/data/stellarr.db"], check=True, capture_output=True)
    subprocess.run(["docker", "start", CONTAINER], check=True, capture_output=True)
    ready(ORACLE)
def start_rust(db, folder, environment="Testing", extra=None):
    global _rust
    stop_rust()
    os.makedirs(folder, exist_ok=True)
    shutil.copy(db, os.path.join(folder, "stellarr.db"))
    env = dict(os.environ, ASPNETCORE_ENVIRONMENT=environment, Rtc__TurnSharedSecret="x", STELLARR_DATA_ROOT=folder,
               **{"ConnectionStrings__Database": f"Data Source={folder}/stellarr.db", "ASPNETCORE_URLS": "http://127.0.0.1:" + RUST_PORT})
    env.update(extra or {})
    log = open(os.path.join(folder, "log.txt"), "w")
    _rust = subprocess.Popen([BINARY], env=env, stdout=log, stderr=subprocess.STDOUT)
    time.sleep(1)
    if _rust.poll() is not None: raise SystemExit("the server did not start: " + open(os.path.join(folder, "log.txt")).read()[-300:])
    ready(RUST)
def stop_rust():
    global _rust
    if _rust:
        _rust.terminate()
        try: _rust.wait(10)
        except Exception: _rust.kill()
        _rust = None
if __name__ == "__main__":
    reset_oracle(sys.argv[1])
    start_rust(sys.argv[1], os.path.join(os.getcwd(), "work", "rust"))
    print("both up")
