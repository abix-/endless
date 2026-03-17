# Current Playable Features

Short version of what is in Endless right now. This is the systems list, not the changelog.

## Town and economy

- Place farms, roads, walls, towers, homes, merchants, mines, lumber mills, and quarries.
- Buildings construct over time instead of appearing instantly.
- Town area expands over time instead of staying fixed to the starting footprint.
- Save, quicksave, quickload, named saves, and autosaves are all live.

Read more: [README.md](../README.md), [economy.md](economy.md), [save-load.md](save-load.md)

## NPC simulation

- NPCs have jobs, homes, energy, combat stats, personality, and explicit activity state.
- Farmers, miners, woodcutters, quarriers, archers, crossbowmen, fighters, raiders, and boats all run through the sim.
- Units rest, heal, flee, haul resources, deposit loot, and return to work without constant babysitting.
- NPCs gain proficiency from doing their job -- farming raises farming skill, dodging raises dodge skill.
- Farming proficiency scales farm growth rate. Dodge proficiency gives a personal projectile miss chance.
- Skills are visible in the inspector and sortable in the roster Prof column.

Read more: [behavior.md](behavior.md), [npc-activity-controller.md](npc-activity-controller.md), [spawn.md](spawn.md)

## Combat and command

- Units can be grouped into squads and sent to map targets.
- Squads support patrol, hold fire, rest discipline, and loot-return thresholds.
- Projectiles, melee, target switching, tower fire, building damage, and destruction aftermath are all live.
- Walls, roads, choke points, and redeployment matter.

Read more: [combat.md](combat.md), [behavior.md](behavior.md), [projectiles.md](projectiles.md)

## Loot and equipment

- Kills generate food, gold, and equipment directly into carrier loot.
- Units carry loot home and deposit gear into town inventory.
- The armory supports equip, unequip, auto-equip, filters, comparisons, and town inventory browsing.
- Gear changes both stats and visuals, so veterans do not stay visually generic.

Read more: [armory-ui.md](armory-ui.md), [combat.md](combat.md), [save-load.md](save-load.md)

## World resources

- Gold mines, trees, and rocks are harvestable world objects.
- Wood and stone are real town resources alongside food and gold.
- Roads, expansion, and remote harvest sites create an actual logistics layer.

Read more: [harvestable-resources.md](harvestable-resources.md), [economy.md](economy.md)

## AI, upgrades, and alternative control

- AI towns build, upgrade, defend, expand, raid, and repopulate through endless mode.
- Builder factions and raider factions play differently.
- Town, military, farmer, and miner upgrades are live.
- The AI Manager can run your own town if you want to play more strategically.
- The built-in LLM player can run a faction.

Read more: [ai-player.md](ai-player.md), [resources.md](resources.md), [llm-player.md](llm-player.md)

## Scale and sandbox

- GPU movement, GPU targeting, GPU projectiles, and instanced rendering are all live.
- Current validation reaches five-figure active-unit gameplay, with larger stress testing beyond that.
- Sandbox and debug test scenes exist if you want to poke at the systems directly.
- HP bar display mode toggle: Off, When Damaged, or Always.
- Microbenchmark CI guardrails catch performance regressions before merge.

Read more: [performance.md](performance.md), [gpu-compute.md](gpu-compute.md), [rendering.md](rendering.md)

## Where To Dive Deeper

- [README.md](../README.md): quick intro, controls, and high-level structure
- [economy.md](economy.md): colony loop, construction, starvation, and resources
- [behavior.md](behavior.md): NPC decision-making and activity flow
- [combat.md](combat.md): combat, destruction, towers, and aftermath
- [ai-player.md](ai-player.md): rival towns and the Player AI Manager
- [armory-ui.md](armory-ui.md): armory and equipment flow
- [harvestable-resources.md](harvestable-resources.md): trees, rocks, wood, stone, and worker roles
- [performance.md](performance.md): scale and current performance model
