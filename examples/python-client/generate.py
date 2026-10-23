"""Generate Python protobuf classes from the public spec (spec/proto)."""

import os
import sys

from grpc_tools import protoc

ROOT = os.path.join(os.path.dirname(__file__), "..", "..", "spec", "proto")
os.makedirs("gen", exist_ok=True)
files = [f"neoshard/v1/{f}" for f in ("models.proto", "gateway.proto")]
code = protoc.main(["protoc", f"-I{ROOT}", "--python_out=gen", *files])
for d in ("gen/neoshard", "gen/neoshard/v1"):
    open(os.path.join(d, "__init__.py"), "a").close()
sys.exit(code)
