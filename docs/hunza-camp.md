# Hunza Camp terrain exception

The reported workaround removes generated LAND records at `(-9, -46)`, `(-9, -47)`,
`(-10, -46)`, and `(-10, -47)`. New `merged_lands.toml` files seed these coordinates in
`cell_ignore_by_plugin["TR_Mainland.esm"]`. Existing configurations retain their settings;
add the entry documented in the README to enable the exception there.

## Source inspection

Inspected the locally installed `TR_Mainland.esm` with header version **26.08a**, described
as **Poison Song — September 2026 update (Hotfix 1)**, and README build **26.09.24**.
This is newer than the Grasping Fortune release in the report; the discontinuity and
generated height drop still reproduce with this installed version.

Among the active 144 content entries, TR is the only height-map contributor at all four coordinates.
Distant Seafloor also supplies texture-only LAND records there, before TR in load order.
The cell at `(-10, -47)` is flat at **-2,048 game units** and has no placed references.
Its entire northern and eastern edges disagree with the neighboring TR LAND heights by
**12,104 units**: those neighboring borders are at **10,056**. The other borders inspected
in the surrounding 4×4 area agree within TR itself.

Comparing the decoded source height maps with the previous full-modlist merged output:

| Cell | Changed vertices | Generated height minus TR height |
| --- | ---: | ---: |
| `(-9, -46)` — Hunza Camp | 4,225 / 4,225 | -3,032 to -2,016 |
| `(-9, -47)` | 4,225 / 4,225 | -5,064 to -3,024 |
| `(-10, -46)` | 4,225 / 4,225 | -6,056 to -5,040 |
| `(-10, -47)` | 129 / 4,225 | 0 to +2,032 |

These values come from decoding the binary VHGT records directly and comparing every vertex,
with `tes3conv` also decoding the four source records. No game data was edited.

## Why the terrain drops

`repair_landmass_seams` averages mismatched cell boundaries and shared corners, including
discontinuities already present in a single source plugin. That changes the first vertex and
boundary heights of the high cells. `calculate_vertex_heights` then encodes row and column
differences into TES3's signed byte deltas, in eight-unit steps. Gradients outside -128…127
are clamped, so the encoded height map cannot retain the unchanged interior heights after
these large boundary changes. The resulting offset error extends across whole rows or cells.

This is a Merged Lands compatibility failure triggered by the source discontinuity; it is
not evidence of another mod changing Hunza Camp. Nearby TR cells contain many
`T_Mw_TerrRockSH_Cliff*` and rock references at the original landscape elevations. That is
consistent with static geometry concealing terrain discontinuities, but author intent and
the exact visual coverage have not been established by inspecting the records alone.

## Scope of the exception

The existing plugin exception mechanism removes these TR records before constructing the
merge reference, plugin diffs, and seam repairs. A guard also removes heightless merged cells
when the ignored winning plugin supplies their original heights. Otherwise, Distant Seafloor's
older texture-only records would survive as generated overrides. OpenMW's
[OpenMW 0.51's `Store<ESM::Land>::load`](https://github.com/OpenMW/openmw/blob/f4bec41444214a7903bebd178389ca22ca13f646/apps/openmw/mwworld/store.cpp)
replaces the entire record; it does not inherit missing height data from TR.
With TR as the only height-map contributor, no generated LAND records remain for these
cells, matching the reported deletion workaround.
Other plugins can still contribute terrain at these coordinates. If another plugin supplies
earlier terrain there, a plugin exception may restore that earlier terrain; use global
`cell_ignore` instead when the entire cell must be excluded from generated output.

The regression test uses synthetic high cells around a low cell and earlier texture-only LAND
records. It checks both OpenMW and classic Morrowind modes with a freshly created config,
verifies the four generated overrides are absent, and confirms ordinary seam repair still works
in neighboring, included cells.
General handling of unencodable repaired gradients is outside this targeted exception.
