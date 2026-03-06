#!/usr/bin/env python3
"""Generic action toolkit for LLM player. Single entry point for all game interactions."""

import requests

API_URL = "http://localhost:15702"

def rpc(method, params=None):
    """Send any JSON-RPC request to the game server."""
    payload = {"jsonrpc": "2.0", "method": method, "params": params or {}, "id": 1}
    resp = requests.post(API_URL, json=payload)
    result = resp.json()
    if "error" in result:
        print(f"ERROR: {result['error']['message']}")
        return None
    return result.get("result")

def parse_toon_value(s):
    """Auto-type a TOON value string."""
    if s == "true": return True
    if s == "false": return False
    if s == "null": return None
    try: return int(s)
    except ValueError: pass
    try: return float(s)
    except ValueError: pass
    return s

def parse_toon_params(args):
    """Parse TOON key:value args into a dict. Falls back to JSON if first arg starts with '{'."""
    import json
    if len(args) == 1 and args[0].startswith("{"):
        return json.loads(args[0])
    params = {}
    for arg in args:
        if ":" not in arg:
            raise ValueError(f"bad param (expected key:value): {arg}")
        key, value = arg.split(":", 1)
        params[key] = parse_toon_value(value)
    return params

if __name__ == "__main__":
    import json, sys
    if len(sys.argv) >= 2:
        method = sys.argv[1]
        params = parse_toon_params(sys.argv[2:]) if len(sys.argv) >= 3 else {}
        result = rpc(method, params)
        if result is not None:
            # TOON responses come back as strings — print raw
            if isinstance(result, str):
                print(result)
            else:
                print(json.dumps(result, indent=2))
    else:
        state = rpc("endless/summary")
        if state:
            for t in state.get("towns", []):
                llm = " [LLM]" if t.get("llm") else ""
                print(f"  Town {t['index']}: {t['name']}{llm} - Food:{t.get('food',0)} Gold:{t.get('gold',0)}")
