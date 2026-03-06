#!/usr/bin/env python3
"""Background polling daemon for LLM player. Writes TOON game state to loop.log every cycle."""

import os
import time
import requests

API_URL = "http://localhost:15702"
INTERVAL = 10
LOG_FILE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "loop.log")

def poll():
    """Fetch summary — returns TOON string or None on error."""
    try:
        resp = requests.post(API_URL, json={"jsonrpc": "2.0", "method": "endless/summary", "params": {}, "id": 1})
        result = resp.json().get("result")
        if isinstance(result, str):
            return result
        return None
    except Exception as e:
        return f"error: {e}"

def main():
    log = open(LOG_FILE, "w", buffering=1)

    def out(msg=""):
        print(msg)
        log.write(msg + "\n")

    cycle = 0

    while True:
        cycle += 1
        toon = poll()

        if toon is None:
            out(f"[cycle {cycle}] No LLM town found. Waiting...")
            time.sleep(INTERVAL)
            continue

        if toon.startswith("error:"):
            out(f"[cycle {cycle}] {toon}")
            time.sleep(INTERVAL)
            continue

        out(f"\n{'='*50}")
        out(f"CYCLE {cycle}")
        out(toon)

        time.sleep(INTERVAL)

if __name__ == "__main__":
    main()
