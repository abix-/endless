#!/usr/bin/env python3
"""Generic action toolkit for LLM player. Thin wrappers over the game's BRP endpoints."""

import requests
import json

API_URL = "http://localhost:15702"

def _rpc(method, params=None):
    """Send a JSON-RPC request to the game server."""
    payload = {"jsonrpc": "2.0", "method": method, "params": params or {}, "id": 1}
    resp = requests.post(API_URL, json=payload)
    result = resp.json()
    if "error" in result:
        print(f"ERROR: {result['error']['message']}")
        return None
    return result.get("result")

# === READ ===

def summary(town=None):
    """Get full game state. Optional town filter."""
    params = {"town": town} if town is not None else {}
    return _rpc("endless/summary", params)

def my_town(state=None):
    """Find the LLM-controlled town from summary. Returns town dict or None."""
    if state is None:
        state = summary()
    for t in state.get("towns", []):
        if t.get("llm"):
            return t
    return None

def my_squads(state=None):
    """Get squad indices for the LLM town."""
    town = my_town(state)
    if not town:
        return []
    return town.get("squads", [])

# === WRITE ===

def set_personality(town, personality):
    """Set AI Manager personality: 'Aggressive', 'Balanced', or 'Economic'."""
    return _rpc("endless/ai_manager", {"town": town, "active": True, "personality": personality})

def set_policy(town, **kwargs):
    """Set town policies. Options: eat_food, archer_aggressive, archer_leash,
    farmer_fight_back, prioritize_healing, farmer_flee_hp, archer_flee_hp,
    recovery_hp, mining_radius."""
    return _rpc("endless/policy", {"town": town, **kwargs})

def buy_upgrade(town, upgrade_idx):
    """Queue an upgrade purchase by index."""
    return _rpc("endless/upgrade", {"town": town, "upgrade_idx": upgrade_idx})

def target_squad(squad, x, y):
    """Send a squad to a position."""
    return _rpc("endless/squad_target", {"squad": squad, "x": x, "y": y})

def build(town, kind, row, col):
    """Place a building. Kinds: Farm, FarmerHome, ArcherHome, Tent, GoldMine,
    MinerHome, CrossbowHome, FighterHome, Road, Wall, Tower, Merchant, Casino."""
    return _rpc("endless/build", {"town": town, "kind": kind, "row": row, "col": col})

def set_time(paused=None, time_scale=None):
    """Pause/unpause or set game speed (0.0-20.0)."""
    params = {}
    if paused is not None:
        params["paused"] = paused
    if time_scale is not None:
        params["time_scale"] = time_scale
    return _rpc("endless/time", params)

if __name__ == "__main__":
    state = summary()
    town = my_town(state)
    if town:
        print(f"LLM town: {town.get('name')} (index {town.get('index')})")
        print(f"  Food: {town.get('food')}, Gold: {town.get('gold')}")
        squads = town.get("squads", [])
        print(f"  Squads: {[s['index'] for s in squads]}")
    else:
        print("No LLM town found. Check the LLM checkbox in the game lobby.")
