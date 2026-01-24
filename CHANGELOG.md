# Changelog

## 2026-01-24
- add separate farmers/guards/raiders sliders in start menu (max 500 each)
- add configurable world size and town count to start menu
- add parallel processing thread-safe state transitions (pending arrivals)
- add co-movement separation reduction (groups move without oscillation)
- add velocity damping for smooth collision avoidance
- add GPU compute shader for separation forces
- add drift detection (working NPCs walk back if pushed off position)
- add flee/leash checks to parallel fighting path
- fix guard patrol route (clockwise perimeter, not diagonal through town)
- fix guard schedule (work 24/7, rest only when energy low, no day/night)
- reduce guard posts from 6 to 4 (corner perimeter)

## 2026-01-23
- add terrain tile inspection on click
- add sprite tiling for water (2x2), dirt (2x2), forest (6 tree types 2x2)
- fix rock sprites (2x2 sprite, variable sprite sizes)
