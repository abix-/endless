"""BRP client for Endless. Usage: python scripts/brp.py <method> [params_json]"""
import urllib.request, json, sys

def brp(method, params=None, port=15702):
    body = {"jsonrpc": "2.0", "id": 1, "method": method}
    if params:
        body["params"] = params
    req = urllib.request.Request(
        f"http://localhost:{port}",
        json.dumps(body).encode(),
        {"Content-Type": "application/json"},
    )
    resp = json.loads(urllib.request.urlopen(req, timeout=5).read())
    if "error" in resp:
        print(f"ERROR: {resp['error']['message']}", file=sys.stderr)
        sys.exit(1)
    return resp["result"]

def perf():
    d = brp("endless/perf")
    lines = d.split("\n")
    for l in lines:
        if not l.startswith("  "):
            print(l)
    timings = {}
    in_t = False
    for l in lines:
        if l.startswith("timings:"):
            in_t = True
            continue
        if in_t and l.startswith("  "):
            parts = l.strip().split(": ")
            if len(parts) == 2:
                try:
                    v = float(parts[1])
                    if v > 0:
                        timings[parts[0].strip('"')] = v
                except ValueError:
                    pass
        elif in_t and not l.startswith("  "):
            in_t = False
    print("\nTop 20:")
    for n, v in sorted(timings.items(), key=lambda x: -x[1])[:20]:
        print(f"  {v:8.2f}ms  {n}")
    eg = sum(v for n, v in timings.items() if "endless::" in n)
    bv = sum(v for n, v in timings.items() if "bevy_" in n and "framepace" not in n)
    print(f"\nEndless total: {eg:.2f}ms | Bevy overhead: {bv:.2f}ms")

def summary():
    print(brp("endless/summary"))

def time(params=None):
    print(brp("endless/time", params))

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python scripts/brp.py <perf|summary|time|METHOD> [params_json]")
        sys.exit(1)
    cmd = sys.argv[1]
    params = json.loads(sys.argv[2]) if len(sys.argv) > 2 else None
    if cmd == "perf":
        perf()
    elif cmd == "summary":
        summary()
    elif cmd == "time":
        time(params)
    else:
        print(brp(cmd, params))
