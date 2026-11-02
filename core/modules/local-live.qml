import QtQuick
import ".."

// A spoken conversation that joins an agent's own text session (Codex or
// Claude Code, the `agent` setting), with speech recognition (Voxtype)
// and the voice (a LocalAI model) on this machine (core/live/local-live.py): what you
// say goes into the session you also type in, and the answer is read aloud.
LiveModule {
  localVoice: true
  appName: setting("agent") === "claude" ? "Claude" : "Codex"
}
