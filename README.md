\# Neroa Compatible Adaptive Bridge



Renderer-independent adaptive web execution bridge for the Neroa spatial browser.



\## Core rule



Neroa owns the browser.



Neither Servo nor Chromium owns navigation, identity, spatial state, lifecycle,

routing, semantic memory, receipts, or input coordinates.



\### Execution lanes



1\. Semantic lane

&#x20;  - Raw HTTP / structured payload

&#x20;  - ASG extraction

&#x20;  - Native Neroa spatial rendering

&#x20;  - No browser renderer where unnecessary



2\. Servo lane

&#x20;  - Preferred live web runtime

&#x20;  - Embedded/offscreen

&#x20;  - Browser-native page interaction

&#x20;  - GPU surface exported into Neroa compositor



3\. Chromium compatibility lane

&#x20;  - CEF / Chromium offscreen compatibility runtime

&#x20;  - Activated only when Servo capability or compatibility is insufficient

&#x20;  - Must not become the architectural browser shell



\## GPU rule



Normal rendering must never use:



GPU -> CPU RGBA buffer -> GPU upload



The renderer produces a GPU surface lease.



The Neroa spatial compositor consumes the shared GPU resource directly.



Platform targets:



\- Windows: D3D12 shared resource

\- Linux: Vulkan external memory / dma-buf

\- macOS: IOSurface / Metal

\- Same-process transitional path: shared GL texture



\## Input path



camera-space ray

\-> spatial node intersection

\-> local node coordinate

\-> normalized UV

\-> physical browser viewport coordinate

\-> Servo or Chromium input event



\## Routing path



network/document signals

\-> semantic eligibility

\-> capability requirements

\-> Servo preferred

\-> Chromium compatibility escalation



A route transition changes only the execution backend.



The spatial node identity remains unchanged.



\## Network interception



Browser requests may be:



\- passed through

\- mirrored to ASG ingestion

\- captured as structured JSON/data

\- blocked by policy

\- used to trigger renderer escalation



\## Lifecycle



Live renderers support:



\- Dormant

\- Frozen

\- Throttled

\- Active



This prevents historical spatial nodes from behaving like thousands of

permanently-running browser tabs.



\## Receipts



Bridge actions emit durable receipts for:



\- node creation

\- route selection

\- renderer transition

\- navigation

\- input forwarding

\- lifecycle transition



\## Bootstrap



```powershell

cargo fmt

cargo test

cargo run

