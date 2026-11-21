// Lays out one message so it fills the panel: the largest size whose lines
// fit, then the narrowest measure that keeps that many lines, so the lines
// come out about equally long. The message arrives through sys.inputs as the
// JSON tree from markdown.rs: strings and style names, never markup to evaluate.

#let input(key, default) = sys.inputs.at(key, default: default)
#let doc = json(bytes(input("doc", "[]")))
#let width = float(input("width", "264")) * 1pt
#let height = float(input("height", "176")) * 1pt
#let margin = 6pt
#let min-size = 12
#let descent = 0.3

#set page(width: width, height: height, margin: margin, fill: white)
#set text(font: ("DejaVu Sans", "Noto Emoji"), weight: "bold", fill: black, hyphenate: false)
#set par(leading: 0.35em, spacing: 0.35em, justify: false, linebreaks: "optimized")

#let styled(kind, body) = {
  if kind == "emph" { emph(body) }
  else if kind == "strong" {
    highlight(fill: black, top-edge: "ascender", bottom-edge: "descender", extent: 0.06em, text(fill: white, body))
  }
  else if kind == "strike" { strike(stroke: 0.1em, body) }
  else if kind == "code" { text(font: ("DejaVu Sans Mono", "Noto Emoji"), body) }
  else if kind == "h1" { text(size: 1.5em, body) }
  else if kind == "h2" { text(size: 1.25em, body) }
  else { body }
}

#let draw(node) = {
  if type(node) == str { node }
  else if node.k == "br" { linebreak() }
  else { styled(node.k, node.c.map(draw).join()) }
}

// Each word with the styles around it, to find the widest one.
#let words(node, wrap: body => body) = {
  if type(node) == str {
    node.split(regex("\s+")).filter(w => w != "").map(w => wrap(w))
  } else {
    node.c.map(child => words(child, wrap: body => wrap(styled(node.k, body)))).flatten()
  }
}

#let body = doc.map(draw).join()
#let all-words = doc.map(node => words(node)).flatten()

#if all-words.len() > 0 {
  context {
    let inner-w = width - 2 * margin
    let inner-h = height - 2 * margin
    let widest(size) = calc.max(..all-words.map(w => measure(text(size: size * 1pt, w)).width))
    let tall(size, w) = measure(block(width: w, text(size: size * 1pt, body))).height
    let fits(size) = widest(size) <= inner-w and tall(size, inner-w) + descent * size * 1pt <= inner-h

    let lo = min-size
    let hi = calc.floor(inner-h / 1pt)
    if not fits(lo) { hi = lo }
    while lo < hi {
      let mid = calc.ceil((lo + hi) / 2)
      if fits(mid) { lo = mid } else { hi = mid - 1 }
    }
    let size = lo

    let goal = tall(size, inner-w)
    let narrow = calc.floor(widest(size) / 1pt)
    let wide = calc.floor(inner-w / 1pt)
    while narrow < wide {
      let mid = calc.floor((narrow + wide) / 2)
      if tall(size, mid * 1pt) <= goal { wide = mid } else { narrow = mid + 1 }
    }

    place(center + horizon, block(width: wide * 1pt, align(center, text(size: size * 1pt, body))))
  }
}
