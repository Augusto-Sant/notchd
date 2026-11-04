// The grid protocol accepts only #rrggbb for the terminal's defaults.
export function hexColor(color) {
    const channel = value => Math.max(0, Math.min(255, Math.round(Number(value) * 255)))
        .toString(16).padStart(2, '0');
    return `#${channel(color.r)}${channel(color.g)}${channel(color.b)}`;
}
