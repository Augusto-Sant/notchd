# Herdr user module

This module shows a summary of a local Herdr session in any notch that lists
`herdr`: agent states, workspace and pane counts. It uses Herdr's snapshot and
event socket; it starts no agents. Its icon is bundled as `herdr.svg`. It runs
as a user module with an existing notchd installation; no daemon release or
rebuild is needed. The `herdr` CLI must be on `PATH`.
The logo is adapted from Herdr's Apache-2.0-licensed artwork; see
[LICENSE-HERDR](LICENSE-HERDR) and the repository's [NOTICE](../../../../NOTICE).

Install it from this repository:

```sh
mkdir -p ~/.config/notchd/modules
cp -r docs/examples/modules/herdr ~/.config/notchd/modules/
```

Then open notchd Settings, add a notch if desired, and select **Herdr** under
that notch's **Loaded modules**. Its settings page has switches for blocked and
done peeks, their duration, and optional desktop notifications. The first
snapshot establishes a baseline and never announces old states. If Herdr stops,
the module waits for it to return.

The module treats `working → idle` as a completed task too. Herdr can report
`idle` instead of `done` when a completion has already been seen in a client;
status events are handled immediately, and periodic snapshots reconcile the
counts if an event is missed.

To show Herdr's terminal interface below the summary, add a terminal in
**Modules › Terminals** with command `herdr` and a separate id such as
`herdr-terminal`. Put `herdr` and `herdr-terminal` in the same notch, in that
order, and disable tabs for that notch. The two expanded views then stack:
metrics above, terminal below. Set the terminal's width to 760 px to align
both views. The terminal starts when the notch is opened; the summary and
notifications continue to work while it is closed.

Equivalent config entries (merge them into your existing config):

```json
{
  "terminals": {
    "herdr-terminal": { "title": "Herdr terminal", "command": "herdr" }
  },
  "herdr-terminal": { "width": 760, "height": 480 },
  "notches": [
    { "id": "Herdr", "modules": ["herdr", "herdr-terminal"], "tabs": false }
  ]
}
```

The settings apply to this module wherever it appears. If several notches list
it, notchd's ordinary peek routing shows the same alert in each. Herdr's
`done` state means a completed agent has not been seen; focusing it clears that
state. This module follows the local default session unless **Session** names
another local session.
