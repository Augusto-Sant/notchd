import QtQuick
import "../.."

// Tier 2: a fixed-size box drawn from the flat command list of
// docs/architecture.md ("Canvas"): rect, circle, line, path (M L Q C Z, in
// capitals), text, and clip ... unclip. Coordinates are design pixels,
// multiplied by `Theme.spacingScale`; colors are tokens or {"mix": [a, b, t]},
// resolved through the Theme, so the drawing follows a theme switch. A bad
// command is skipped, never a crash. Redraws when the node or the theme
// changes (`animate` only says the core may stream views; each `view` is a
// new node).
PrimBase {
  id: prim
  readonly property real scaleFactor: Theme.spacingScale
  implicitWidth: prim.px(prim.node.w, 64)
  implicitHeight: prim.px(prim.node.h, 64)

  onNodeChanged: surface.requestPaint()
  Connections {
    target: Theme
    function onProviderChanged() { surface.requestPaint() }
  }

  Canvas {
    id: surface
    anchors.fill: parent
    renderTarget: Canvas.Image
    onWidthChanged: requestPaint()
    onHeightChanged: requestPaint()

    // `M0 0 L10 10 Q5 5 2 2 C1 1 2 2 3 3 Z` as [[cmd, numbers...], ...].
    function segments(d) {
      var out = []
      var re = /([MLQCZ])|(-?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?)/g
      var m
      while ((m = re.exec(String(d))) !== null) {
        if (m[1]) out.push([m[1]])
        else if (out.length > 0) out[out.length - 1].push(parseFloat(m[2]))
      }
      return out
    }

    function trace(ctx, d, k) {
      var segs = surface.segments(d)
      for (var i = 0; i < segs.length; i++) {
        var s = segs[i]
        // An implicit repeat of a command (`L 1 2 3 4`) draws every pair.
        var arity = s[0] === "M" || s[0] === "L" ? 2 : s[0] === "Q" ? 4 : s[0] === "C" ? 6 : 0
        if (s[0] === "Z") { ctx.closePath(); continue }
        for (var j = 1; j + arity <= s.length; j += arity) {
          var a = s.slice(j, j + arity).map(function(v) { return v * k })
          if (s[0] === "M") ctx.moveTo(a[0], a[1])
          else if (s[0] === "L") ctx.lineTo(a[0], a[1])
          else if (s[0] === "Q") ctx.quadraticCurveTo(a[0], a[1], a[2], a[3])
          else ctx.bezierCurveTo(a[0], a[1], a[2], a[3], a[4], a[5])
        }
      }
    }

    function drawCommand(ctx, c, k) {
      var op = prim.num(c.opacity, 1)
      switch (c.c) {
      case "clip":
        ctx.save()
        ctx.beginPath()
        ctx.roundedRect(c.x * k, c.y * k, c.w * k, c.h * k, prim.num(c.r, 0) * k, prim.num(c.r, 0) * k)
        ctx.clip()
        return
      case "unclip":
        ctx.restore()
        return
      }
      ctx.save()
      ctx.globalAlpha = prim.clamp01(op)
      ctx.beginPath()
      if (c.c === "rect") {
        var r = prim.num(c.r, 0) * k
        if (r > 0) ctx.roundedRect(c.x * k, c.y * k, c.w * k, c.h * k, r, r)
        else ctx.rect(c.x * k, c.y * k, c.w * k, c.h * k)
      } else if (c.c === "circle") {
        ctx.arc(c.x * k, c.y * k, c.r * k, 0, Math.PI * 2, false)
      } else if (c.c === "line") {
        ctx.moveTo(c.x1 * k, c.y1 * k)
        ctx.lineTo(c.x2 * k, c.y2 * k)
      } else if (c.c === "path") {
        surface.trace(ctx, c.d, k)
      } else if (c.c === "text") {
        ctx.font = Math.round(prim.fontPx(c.size, "body")) + "px \"" + Theme.font.family + "\""
        ctx.textBaseline = "top"
        ctx.fillStyle = prim.color(c.tone === undefined ? "foreground" : c.tone)
        ctx.fillText(String(c.text), c.x * k, c.y * k)
        ctx.restore()
        return
      } else {
        ctx.restore()
        return
      }
      if (c.fill !== undefined && c.fill !== null) {
        ctx.fillStyle = prim.color(c.fill)
        ctx.fill()
      }
      // A line has no inside: its `stroke` is required, the others' optional.
      if (c.stroke !== undefined && c.stroke !== null) {
        ctx.strokeStyle = prim.color(c.stroke)
        ctx.lineWidth = Math.max(0.5, prim.num(c.width, 1) * k)
        ctx.lineCap = c.cap === "round" || c.cap === "square" ? c.cap : "butt"
        ctx.lineJoin = c.join === "round" || c.join === "bevel" ? c.join : "miter"
        ctx.stroke()
      }
      ctx.restore()
    }

    onPaint: {
      var ctx = getContext("2d")
      ctx.reset()
      var cmds = prim.node.cmds
      if (!cmds) return
      var depth = 0
      for (var i = 0; i < cmds.length; i++) {
        var c = cmds[i]
        if (c === null || typeof c !== "object") continue
        try {
          if (c.c === "clip") depth++
          else if (c.c === "unclip") { if (depth === 0) continue; depth-- }
          surface.drawCommand(ctx, c, prim.scaleFactor)
        } catch (e) {
          console.warn("canvas: skipped a `" + c.c + "` command: " + e)
        }
      }
      // An unclosed clip must not leak into the next paint.
      while (depth-- > 0) ctx.restore()
    }
  }
}
