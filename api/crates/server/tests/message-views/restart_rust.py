import sys, os, subprocess
sys.path.insert(0, '.')
import harness
# a server left running by an earlier script holds the port
out = subprocess.run(["lsof", "-tiTCP:" + harness.RUST_PORT, "-sTCP:LISTEN"], capture_output=True, text=True).stdout.split()
for pid in out: os.kill(int(pid), 15)
harness.start_rust(sys.argv[1], os.path.join(os.getcwd(), "work", "rust"))
# detach so the next script finds it running
harness._rust = None
print("rust up")
