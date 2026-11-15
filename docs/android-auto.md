# K-OMSI Auto vehicle display

This Windows prototype provides an Android Auto inspired app launcher, a custom game-road
navigator, a YouTube link player and a permanent CCTV button. It is not phone projection.
The browser screen follows the user-provided Car App Templates Design Kit references:
charcoal surfaces, pale-blue pill buttons, SVG icons, stateful switches, a dark navigation
map, compact turn/arrival cards and a map-first dashboard. Korean labels are used.

## Responsive UI

The default page is the dashboard, with the actual game map and next-stop/YouTube cards.
The sidecar's render dimensions control layout; this is not a uniformly scaled 800px page.

| Reference resolution | Layout |
| --- | --- |
| 748×450, 800×480 | Bottom rail, map beside dashboard cards |
| 880×480, 1120×480 | Vertical rail, larger map |
| 1280×480, 1920×584 | Vertical rail; app list and settings retain a map on the right |
| 790×686, 800×880 | Bottom rail; map above two dashboard cards |

The reference UI range starts at 748×450. Full navigation uses the map area, and video
uses the content area without a side map. Apps have only the actual four implemented entries,
not dummy phone/weather/messaging apps. Active, pressed and keyboard-focus states are defined.
Settings support a brightness slider, a heading-follow switch and a dark/light map switch;
the compass control and heading switch are synchronized. These settings last for this display
session. Overflowing settings/lists support texture-pointer drag scrolling with inertia.
Dragging a setting row does not toggle it accidentally; sliders keep their normal interaction.

Reference screenshot colors/proportions were interpreted in CSS; original Figma font and
spacing tokens were not supplied. No bitmap screenshots or kit sample maps are embedded.

## Enable

Build the modified K-OMSI source. Existing installed binaries are not modified by this patch.
Copy the example sidecar next to the matching `.bus`:

- `HigerHypers1611P.bus` → `HigerHypers1611P.android-auto.cfg`
- `HigerHypers1611P_hiprest.bus` → `HigerHypers1611P_hiprest.android-auto.cfg`

Re-place the vehicle after changing the sidecar; live model reload is not supported.
Click the visible door CCTV screen to open Auto; the camera icon in the rail returns to CCTV.
The original screen visibility (power/gear conditions) remains authoritative.
No sidecar means no adaptation; AI vehicle types are not adapted.

The six lines following `[android_auto]` are mesh path, original camera texture,
zero-based occurrence of that texture, width and height. Use 1600×960 for the Higer high-density test (800×480 logical layout).
Dimensions are limited to 64–2048. Missing meshes/materials and existing target overrides or
mouse events disable adaptation with a warning, leaving CCTV intact.

## Browser and YouTube

The opt-in vehicle display uses an installed Edge or Chrome through a dedicated headless
process, with a fresh temporary profile. No browser is downloaded and the normal user profile
is not used. Windows Job Object lifetime management closes the browser subprocesses when the
owner closes. `K_OMSI_AUTO_BROWSER` can specify a browser executable; `K_OMSI_AUTO_NATIVE`
forces the earlier lightweight display for diagnostics. Without an available browser, or
if its connection fails, the lightweight screen is used and reports `Browser unavailable`.
The lightweight fallback does not support video or the new canvas map.

Copy a YouTube video link before/while using the game, open YouTube, then press
**복사한 링크 열기**. This explicit button is the only point where the system clipboard is read.
Standard watch, youtu.be, shorts, live and embed video links are accepted; arbitrary hosts
and invalid IDs are rejected. The official youtube-nocookie.com embedded player handles the
video, controls and browser audio. Click its play button. There is no video downloading,
YouTube search UI, login/account library or smartphone connection in this prototype.
Video-specific embed restrictions and normal network/player errors still apply.
Switching away from YouTube, closing the video or returning to CCTV removes the player,
stopping video and audio. This avoids hidden playback in CCTV mode.

A local loopback-only HTTP page supplies a real origin/referrer for the embed. Framebuffer
screenshots and trusted pointer events travel over the browser's local CDP connection.
The iframe is sandboxed without top-level navigation. Browser work runs outside the simulation
thread, with a single latest-frame slot and bounded event/pointer queues. Screenshots are
requested at at most about 10 fps, with additional encoding/transfer cost: smooth 30/60 fps
video and GPU texture sharing are not implemented. In CCTV mode capture is suspended.

## Custom map

The map consumes the existing Navigator's actual lane geometry, computed route, next turn,
street, next-stop distance and rerouting state. It uses game world metres, not Google Maps,
latitude/longitude or an invented road background. Navigation calculations continue for the
vehicle screen even with the HUD minimap hidden. Geometry is published at most twice a second,
limited to 2500 nearby display roads, 150 route lanes and 20 stops; current vehicle position and
heading continue to follow the vehicle snapshot. Zoom and heading/north-up controls are local
to the screen. There is no separate destination search or new route planner.

The existing duty/HUD Navigator remains responsible for path correctness and recalculation.
The browser map does not change the duty or IBIS. Without available road data it shows a
loading/unavailable state. Its first release is a 2D road/route display, without building
models, map imagery or Google branding.

## Higer inspection and CCTV path

Both .bus variants use `model/model_1611P.cfg` and declare nine reflection cameras.
`Interior/doorcamerascreen.o3d` names `reflexion8.bmp`; camera 8 is
`[add_camera_reflexion_2]`, position (0.187437, -0.197729, 2.86691), FOV 70, yaw 30,
pitch -40, radius 25. `Interior/rearcamerascreen.o3d` names `reflexion7.bmp`.
The door screen occurs twice (model lines 3697, 4166), with visibility respectively
`elec_busbar_main = 1` and `antrieb_getr_gangwahl = 0`; both conditions are preserved.
`texture/reflexion0.bmp`–`reflexion8.bmp` are disk placeholders. Actual material names are
inside the .o3d meshes, not separate [matl] declarations in model.cfg.

`omsi-vehicle/src/vehicle.rs` parses the cameras; `omsi-app/src/camera_util.rs::render_mirrors`
draws them; `omsi-scene/src/staging.rs::mirror_texture` supplies render textures;
`omsi-scene/src/vehicles.rs::vehicle_slot_material` resolves `reflexionN.bmp`.
The adapter in `omsi-sim/src/vehicle/android_auto.rs` adds a material variant: base is the
original camera; item is a newly allocated script/HTML texture. Existing texture indices,
cameras and mesh visibility are preserved. The picker bypasses hidden HTML in CCTV mode.
The browser backend is `omsi-htmlengine/src/browser.rs`; the new page is `browser.html`.
Navigator exports road data in `omsi-app/src/navigator.rs::publish_vehicle_map`.

## Validation and remaining simulator checks

- Lightweight HTML engine: 50 tests pass; benchmark and explicit browser test are excluded.
- Explicit headless Edge smoke test passes at all eight reference resolutions: dashboard,
  app/nav/settings/YouTube selection, rail orientation, wide-map visibility, viewport bounds,
  frame dimensions, pointer scrolling, synchronized settings, invalid link rejection, CCTV
  event and player removal. Screenshots are exported with synthetic test road/vehicle data.
- A public official YouTube sample was loaded over the network and its play button clicked;
  captured frames showed the embed thumbnail and then the video content. Speaker output
  has not been independently verified.
- Whole app `cargo check -p omsi-app --offline` passes.
- Earlier installed-Higer tests verify both .bus camera counts, real mesh material and
  preservation of both screen definitions. The user separately confirmed existing CCTV in
  OMSI 2 and K-OMSI.

The resolution-matrix previews use synthetic test geometry. The separate Grundorf cartography previews use extracted installed-map data, with an inspection camera; neither is an in-game capture.
A built game session still needs to verify actual vehicle UV orientation, route synchronization,
CCTV switching during driving, audio output, browser performance and vehicle replacement.
No source assets from the Higer vehicle or the ETS2 DLL are redistributed.

## Map cartography update

Map-only conversion is shared in `omsi-sim/src/traffic/cartography.rs`. It reuses the
Navigator's lane filtering and carriageway grouping, preserving medians and separated
heights. Routing and driving geometry are not simplified. Display polylines use metre-based
Douglas-Peucker simplification. Nearby roads are cached by network version.

The canvas draws road casings and surfaces by elevation band, actual street-sign-derived
road names and named stops with collision avoidance. Camera position, zoom and heading
interpolate smoothly, including heading wraparound. Navigation geometry is sent only when
its revision changes; vehicle snapshots retain the last geometry.

Optional `openomsi-realmap` packages supply land-use triangles, water/rail/border lines and
attribution. No such package was found in the installed maps. Individual building footprints
are not extracted, and no invented terrain or buildings are added. This is a local canvas
renderer inspired by the reference, not an integration of the Mapbox SDK.

Read-only base-map inspection (without chrono patches or a running simulator):

```powershell
cargo run -p omsi-scene --example cartography_inspect --offline -- OMSI_ROOT Grundorf OUTPUT.json
```

Grundorf inspection found 19 tiles, 894 traffic lanes including non-road lanes, 260 street
lanes, 219 display roads, 32 street signs and 14 named stops. The explicit browser test uses
this extracted JSON and checks rendered labels, zoom-dependent label suppression and
359-to-1-degree camera rotation. Both browser tests pass sequentially: actual map and all
eight UI resolutions. Three cartography unit tests also pass.

`Grundorf-before.png` is a controlled raw-lane comparison using the same renderer;
`Grundorf-after.png` uses grouped roads and actual labels. The preview camera is positioned
for inspecting labels, not synchronized to a live vehicle.

## Higer high-density preset
The Higer example now renders 1600×960 with an 800×480 CSS viewport (device scale 2). Even dimensions at least 1496×900 use double-density rendering. Other reference sizes retain scale 1. Texture mouse coordinates are divided by the same scale before CDP dispatch. Restart the test build and re-place the vehicle; increasing resolution improves sharpness, not physical font size, and increases capture cost.

## Actual CCTV UV window (supersedes the rectangular Higer preset)
The installed Higer screen material uses U approximately 0..1 and V 0.20368588..0.79631495. Existing mirror_texture allocates a square texture at the configured mirror size. The Higer sidecar now requests 1600×1600. The adapter reads the selected material UV bounds and passes them as internal page metadata; the browser renders only the visible rectangle, pads the full texture and subtracts the rectangle origin for input. CCTV mesh UVs, material and cameras stay unchanged. Its visible region is approximately 1600×948, with a 799×474 logical viewport after safe integer rounding. The installed-vehicle test checks both variants and the browser test verifies full rail visibility and pointer events. Rechecking the game after restart remains necessary. Native fallback does not yet fit the cropped UV window.

## Browser startup recovery
Browser start and connection response waits are now 30 seconds and the worker retries up to three isolated sessions. After a modern frame was delivered, a connection failure retains that frame instead of switching to the legacy page; it may appear frozen until recovery. Reconnection resets browser page state. The original Grundorf 76 test logged socket timeout 10060, which triggered the legacy fallback. No UI design rollback occurred.

## 30 fps target stream (current transport)

The backend now uses Page.startScreencast instead of requesting Page.captureScreenshot
for every frame. Default JPEG quality 90, target 30 fps. Set K_OMSI_AUTO_CAPTURE=png for
lossless streaming or K_OMSI_AUTO_FPS=1..60 for a target rate. The deadline includes work,
so processing time is no longer followed by a fixed extra 100 ms sleep. Stream events are
acknowledged and only the newest received frame is retained. CCTV stops the screencast;
returning to Auto restarts it. Browser audio and player teardown are unchanged.

Chromium sends logical viewport pixels (about 799x474 for Higer), expanded into the existing
1600x1600 UV window. This trades the previous HiDPI screenshot sharpness for throughput;
it is not GPU shared-texture rendering or native 1600px video. Exact 2x expansion has a
cheap pixel/row duplication path. Neither game FPS nor actual video FPS is guaranteed.

A six-second animated synthetic browser test produced 157 frames, 26.15 fps on this machine
with target 30; the earlier per-request capture experiment was around 8–10 fps. This is not
an isolated game benchmark or a YouTube playback measurement. Auto capture log lines record
actual frame throughput, work time and encoded bytes every five active seconds. The test also
checks that CCTV suspends frame production. Higer UV/click browser test and all 50 normal
HTML engine tests pass. User separately confirmed video/audio and their stop on CCTV before
this transport change; the new build still needs the same in-game confirmation.

## CCTV background playback (current behavior)

CCTV now switches the visible vehicle material only. The YouTube iframe, selected page,
browser stream and audio stay active. Returning to Auto restores the same page/player.
The explicit video stop button removes the iframe. Switching to other app pages still has
the previous stop behavior. Keeping the stream active also keeps its capture cost in CCTV.
This supersedes earlier instructions saying CCTV stops playback or capture.

User follow-up: audio still cuts out when switching to CCTV in their game session.
The user accepted that behavior and requested moving on; audible background playback
is not verified and further audio investigation is deferred. Preserving the iframe and
stream in browser checks does not establish continuous speaker output in the simulator.

## Driving telemetry regression check

`browser_driving_route_and_next_stop_updates` passes with a dedicated headless Edge
session. Synthetic consecutive snapshots verify position/heading convergence while
retaining navigation geometry, next-stop name and arrival updates, replacement route
geometry, turn distance, next-stop distance, rerouting text and north-up convergence.
The normal HTML engine suite also passes (50 tests). This is browser integration
coverage, not a real Grundorf driving or timetable progression confirmation.

## Cropped display bezel inset

For cropped material UV windows, the browser page now has logical insets of
8 px top/left, 32 px right and 24 px bottom. The user's game capture showed the
close button and bottom rail extending beneath the physical bezel. This keeps
controls inside that bezel while preserving CCTV UVs and pointer mapping.
The Higer UV browser check covers YouTube close/paste and rail button bounds
and pointer dispatch. Actual game fit still requires checking the new build.
Capture resolution and stream quality are unchanged by this fit adjustment.

## YouTube playlist links

The browser now retains validated `list` and positive `index` values from watch
and youtu.be links, and accepts playlist-only links using the videoseries embed.
Single-video links keep their existing behavior. A dedicated browser test covers
all three forms, parameter preservation, invalid hosts/IDs and explicit stop.
This checks generated player URLs; actual playlist availability and automatic
progression depend on YouTube and need an in-game playback check.

## Upcoming stop panel

Full navigation shows up to three stops beginning at the timetable's nextIndex,
with planned arrival times when supplied. Without a timetable index, the current
IBIS stop name is matched to its stop list; unknown progression is not invented.
An approach hint appears within 150 m, and a nearby hint within 30 m. A nearby
bus below 3 km/h with open doors shows a door-open hint, not proof of a served stop.
These hints use Navigator distance and vehicle telemetry; no speech is added.
The panel participates in map label collision avoidance and remains hidden on
the dashboard. Browser regression checks cover progression, terminal stop,
door-open proximity and missing duty data. Real Grundorf driving remains unverified.

User subsequently confirmed automatic updates during actual driving and supplied
game captures showing the stop panel and door-open proximity hint. This confirms
the reported in-game automatic updates; off-route recalculation is not separately
confirmed by that report.

## YouTube error diagnostics

The IFrame API now reports player errors in the status line (2/5/100/101/150/153)
and marks actual playback as playing. The player is destroyed on explicit stop or
app departure, retaining the previous CCTV preservation behavior.

For user video `6-8iYWkIgRY`, both youtube-nocookie.com and youtube.com embeds in
the same isolated browser returned error 150, with player data errorCode `auth`.
The official sample `M7lc1UVf-VE` reached playing state 1 in that environment.
Public watch metadata said playableInEmbed=true and oEmbed returned 200, so those
metadata checks were insufficient to establish actual playback. The precise
account/rights reason behind YouTube's rejection was not established. No account
login, embed restriction bypass or domain switch is implemented.
