#!/usr/bin/env python3
"""Background polling daemon for LLM player. Writes game state to loop.log every cycle."""

import json
import os
import time
import requests

API_URL = "http://localhost:15702"
INTERVAL = 10
LOG_FILE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "loop.log")

def poll():
    try:
        resp = requests.post(API_URL, json={"jsonrpc": "2.0", "method": "endless/summary", "params": {}, "id": 1})
        return resp.json().get("result", {})
    except Exception as e:
        return {"error": str(e)}

def find_my_town(state):
    for t in state.get("towns", []):
        if t.get("llm"):
            return t
    return None

def main():
    log = open(LOG_FILE, "w", buffering=1)

    def out(msg=""):
        print(msg)
        log.write(msg + "\n")

    cycle = 0
    my_town_idx = None

    while True:
        cycle += 1
        state = poll()

        if "error" in state:
            out(f"[cycle {cycle}] ERROR: {state['error']}")
            time.sleep(INTERVAL)
            continue

        # Auto-discover town on first cycle
        if my_town_idx is None:
            my_town = find_my_town(state)
            if not my_town:
                out(f"[cycle {cycle}] No LLM town found. Waiting...")
                time.sleep(INTERVAL)
                continue
            my_town_idx = my_town["index"]
            out(f"Discovered LLM town: {my_town.get('name')} (index {my_town_idx})")

        gt = state.get("game_time", {})
        my = next((t for t in state.get("towns", []) if t["index"] == my_town_idx), None)
        if not my:
            time.sleep(INTERVAL)
            continue

        my_faction = my.get("faction")
        my_fdata = next((f for f in state.get("factions", []) if f["faction"] == my_faction), {})

        out(f"\n{'='*50}")
        out(f"CYCLE {cycle} [Day {gt.get('day','?')} {gt.get('hour','?'):02d}:{gt.get('minute','?'):02d}]")
        out(f"MY STATUS: Food={my.get('food',0)}, Gold={my.get('gold',0)}, Alive={my_fdata.get('alive',0)}, Dead={my_fdata.get('dead',0)}")

        # Squads
        squads = my.get("squads", [])
        if squads:
            out("SQUADS:")
            for s in squads:
                target = s.get("target")
                t_str = f"-> ({target['x']:.0f},{target['y']:.0f})" if target else "idle"
                out(f"  #{s['index']}: {s['members']} members {t_str}")

        # Enemies
        out("ENEMIES:")
        seen_factions = set()
        for town in state.get("towns", []):
            if town["index"] == my_town_idx:
                continue
            fac = town.get("faction")
            if fac is None or fac in seen_factions:
                continue
            seen_factions.add(fac)
            fdata = next((f for f in state.get("factions", []) if f["faction"] == fac), {})
            center = town.get("center", {})
            out(f"  {town.get('name','?')} (f{fac}): {fdata.get('alive',0)} alive, {fdata.get('dead',0)} dead @ ({center.get('x',0):.0f},{center.get('y',0):.0f})")

        time.sleep(INTERVAL)

if __name__ == "__main__":
    main()
