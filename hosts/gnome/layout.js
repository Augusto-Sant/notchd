// The GNOME actor includes the outline's shoulders; Position.place works on
// the card body. Keep that conversion independent of Clutter for geometry
// tests against the same Position.js used by the QML host.
export function placeCard(Position, {edge, offset, monitor, insets, along, depth, shoulder, snap}) {
    const vertical = Position.isVertical(edge);
    const spot = Position.place({edge, offset, width: monitor.width, height: monitor.height,
        cardWidth: vertical ? depth : along, cardHeight: vertical ? along : depth,
        insets, snap});
    return {spot, target: {
        x: Math.round(monitor.x + spot.x - (vertical ? 0 : shoulder)),
        y: Math.round(monitor.y + spot.y - (vertical ? shoulder : 0)),
        w: vertical ? depth : along + 2 * shoulder,
        h: vertical ? along + 2 * shoulder : depth,
    }};
}
