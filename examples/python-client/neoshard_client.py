#!/usr/bin/env python3
"""Minimal NeoShard client in Python, written only from the public spec
(spec/proto/*.proto, spec/openapi.yaml, spec/gateway.md) — no neoshard-core.

    python generate.py                      # protobuf classes from spec/proto
    python neoshard_client.py --server http://127.0.0.1:8080 --login bob --channel 123

It logs in over REST, connects to the Gateway (protobuf over WebSocket with
per-frame zstd), keeps the heartbeat going, prints messages of a channel and
sends whatever you type. SPDX-License-Identifier: CC0-1.0 (example code).
"""

import argparse
import asyncio
import getpass
import json
import random
import sys
import urllib.request

import websockets
import zstandard

sys.path.insert(0, "gen")
from neoshard.v1 import gateway_pb2 as gw  # noqa: E402

ZSTD_MAGIC = b"\x28\xb5\x2f\xfd"
_zd = zstandard.ZstdDecompressor()


def rest(server, method, path, token=None, body=None):
    req = urllib.request.Request(
        f"{server}/api/v1{path}",
        method=method,
        data=None if body is None else json.dumps(body).encode(),
        headers={"content-type": "application/json", **({"authorization": f"Bearer {token}"} if token else {})},
    )
    with urllib.request.urlopen(req) as r:
        return json.loads(r.read() or b"null")


def decode(data: bytes) -> gw.GatewayFrame:
    # Frames are self-describing: zstd magic → compressed (spec/gateway.md).
    if data[:4] == ZSTD_MAGIC:
        data = _zd.decompress(data, max_output_size=8 * 1024 * 1024)
    f = gw.GatewayFrame()
    f.ParseFromString(data)
    return f


def frame(**kw) -> bytes:
    return gw.GatewayFrame(**kw).SerializeToString()


async def run(server, token, channel_id):
    discovery = json.loads(urllib.request.urlopen(f"{server}/.well-known/neoshard").read())
    url = discovery["gateway"] + "?v=1&compress=zstd"
    async with websockets.connect(url, max_size=8 * 1024 * 1024) as ws:
        hello = decode(await ws.recv()).hello
        interval = hello.heartbeat_interval_ms / 1000
        await ws.send(frame(identify=gw.Identify(token=token, properties=gw.ClientProperties(client="python-example/0.1"))))
        state = {"seq": 0}

        async def heartbeat():
            await asyncio.sleep(interval * random.random())
            while True:
                await ws.send(frame(heartbeat=gw.Heartbeat(last_seq=state["seq"])))
                await asyncio.sleep(interval)

        async def typing_loop():
            loop = asyncio.get_running_loop()
            while True:
                line = await loop.run_in_executor(None, sys.stdin.readline)
                if not line:
                    return
                if line.strip():
                    await loop.run_in_executor(None, lambda: rest(server, "POST", f"/channels/{channel_id}/messages", token, {"content": line.strip()}))

        tasks = [asyncio.create_task(heartbeat()), asyncio.create_task(typing_loop())]
        try:
            async for raw in ws:
                f = decode(raw)
                kind = f.WhichOneof("kind")
                if kind == "ready":
                    print(f"* вошли как {f.ready.user.username}, гильдий: {len(f.ready.guilds)}")
                    for m in rest(server, "GET", f"/channels/{channel_id}/messages?limit=10", token):
                        print(f"[{m.get('author', {}).get('username', '?')}] {m.get('content', '')}")
                elif kind == "dispatch":
                    for d in f.dispatch.items:
                        state["seq"] = d.seq
                        ev = d.event
                        if ev.WhichOneof("kind") == "message_create" and ev.message_create.channel_id == channel_id:
                            print(f"[{ev.message_create.author.username}] {ev.message_create.content}")
                elif kind == "error":
                    print(f"! gateway: {f.error.message}")
                    return
        finally:
            for t in tasks:
                t.cancel()


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--server", default="http://127.0.0.1:8080")
    p.add_argument("--login", required=True)
    p.add_argument("--password")
    p.add_argument("--channel", type=int, required=True)
    a = p.parse_args()
    auth = rest(a.server, "POST", "/auth/login", body={"login": a.login, "password": a.password or getpass.getpass("Пароль: ")})
    if auth.get("mfa"):
        auth = rest(a.server, "POST", "/auth/mfa/totp", body={"ticket": auth["ticket"], "code": input("Код 2FA: ")})
    asyncio.run(run(a.server, auth["token"], a.channel))


if __name__ == "__main__":
    main()
