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

if __name__ == "__main__":
    import json, sys
    if len(sys.argv) >= 2:
        method = sys.argv[1]
        params = json.loads(sys.argv[2]) if len(sys.argv) >= 3 else {}
        result = rpc(method, params)
        if result is not None:
            print(json.dumps(result, indent=2))
    else:
        state = rpc("endless/summary")
        if state:
            for t in state.get("towns", []):
                llm = " [LLM]" if t.get("llm") else ""
                print(f"  Town {t['index']}: {t['name']}{llm} - Food:{t.get('food',0)} Gold:{t.get('gold',0)}")
