// The preferences are a separate process. Its session-bus name disappears
// even when it exits without sending the normal settings-close message.
export const PREFS_BUS_NAME = 'org.notchd.GnomePreferences';
export const PREFS_REQUEST_KEY = 'daemon-preferences-request';

// A GSettings timestamp survives the new process starting, but an old
// request cannot turn a later, independently opened window into a daemon one.
export function requestedPreferences(requestedAt, now) {
    return requestedAt > 0 && now >= requestedAt && now - requestedAt <= 60;
}

export function preferencesGone(wasPresent, settingsOpen) {
    return wasPresent && settingsOpen;
}
