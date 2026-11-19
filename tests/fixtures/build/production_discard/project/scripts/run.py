import json
import subprocess
import sys

target, manifest_path, separator, *arguments = sys.argv[1:]
assert separator == "--"
with open(manifest_path, encoding="utf-8") as file:
    executable = json.load(file)["executable"]
prefix = [] if target == "native" else ["wasmtime", "run", "-W", "max-wasm-stack=524288"]
sys.exit(subprocess.run([*prefix, executable, *arguments]).returncode)
