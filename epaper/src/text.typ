// Lays out one message so it fills the panel: the largest size whose lines
// fit, then the narrowest measure that keeps that many lines, so the lines
// come out about equally long. The message arrives as a string through
// sys.inputs and is never evaluated as markup.

#let input(key, default) = sys.inputs.at(key, default: default)
#let message = input("text", "").trim()
#let width = float(input("width", "264")) * 1pt
#let height = float(input("height", "176")) * 1pt
#let ink = if input("ink", "black") == "red" { rgb(200, 0, 0) } else { black }
#let margin = 6pt
#let min-size = 12
#let descent = 0.3

#set page(width: width, height: height, margin: margin, fill: white)
#set text(font: "DejaVu Sans", weight: "bold", fill: ink, hyphenate: false)
#set par(leading: 0.35em, spacing: 0.35em, justify: false, linebreaks: "optimized")

#let lines = message.split("\n").map(line => line.split(regex("\s+")).filter(w => w != "").join(" "))
#let words = message.split(regex("\s+")).filter(w => w != "")
#let body = lines.map(line => [#line]).join(linebreak())

#if words.len() > 0 {
  context {
    let inner-w = width - 2 * margin
    let inner-h = height - 2 * margin
    let widest(size) = calc.max(..words.map(w => measure(text(size: size * 1pt, w)).width))
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
