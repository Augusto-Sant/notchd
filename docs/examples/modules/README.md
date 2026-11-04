# User module examples

Each folder is a user module you can copy into `~/.config/notchd/modules/`.
The [user module guide](../../modules.md#user-modules) explains the manifest,
JSON messages and view primitives. Saving a module reloads it; no daemon build
is needed.

| Example | What it demonstrates |
|---|---|
| [Herdr](herdr/) | A complete integration: read another tool's snapshot and events, show live metrics, peek on agent status changes, and pair the module with a terminal in one notch. |
| [Counter](counter/) | A shell script with buttons and local state. |
| [CPU](cpu/) | A Python module that subscribes to the resources provider. |
| [Greeting](greeting/) | A Node module with user settings. |

For the full Herdr layout and installation steps, start with its [README](herdr/README.md).
