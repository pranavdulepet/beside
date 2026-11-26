"""Render the default BesideMark curve at Dock, site and menu bar sizes."""
import json
from pathlib import Path

from PIL import Image, ImageDraw

BUTTER = "#F5E5A6"
INK = "#48538F"
ICONS = Path("src-tauri/icons")


def signature(size, ink):
    points = [(24*x, 15*y) for x, y in json.loads(Path("src/signature.json").read_text())]
    left, top = map(min, zip(*points))
    right, bottom = map(max, zip(*points))
    scale = size / max(right-left+2.8, bottom-top+2.8)
    def pixel(x, y):
        return (round(size/2 + (x-(left+right)/2)*scale),
                round(size/2 + (y-(top+bottom)/2)*scale))

    # Same midpoint quadratic Béziers and rounded ends as the 25-point sheet mark.
    path = [pixel(*points[0])]
    start = points[0]
    for control, following in zip(points, points[1:]):
        end = tuple((a+b)/2 for a,b in zip(control, following))
        for step in range(1, 25):
            t = step/24
            path.append(pixel(*((1-t)**2*a+2*(1-t)*t*b+t*t*c for a,b,c in zip(start,control,end))))
        start = end
    path.append(pixel(*points[-1]))
    canvas = Image.new("RGBA", (size, size))
    draw = ImageDraw.Draw(canvas)
    width = round(size*1.9/25)
    draw.line(path, fill=ink, width=width, joint="curve")
    for x,y in (path[0], path[-1]):
        r = width/2
        draw.ellipse((x-r, y-r, x+r, y+r), fill=ink)
    return canvas


if __name__ == "__main__":
    mark = signature(1024, INK)
    mark.resize((256,256), Image.Resampling.LANCZOS).save("public/materials/beside.png")
    signature(512, "black").resize((36,36), Image.Resampling.LANCZOS).save(ICONS/"tray.png")
    tile = Image.new("RGBA", (1024,1024))
    ImageDraw.Draw(tile).rounded_rectangle((48,48,976,976), radius=204, fill=BUTTER)
    tile.alpha_composite(mark.resize((748,748), Image.Resampling.LANCZOS), (138,138))
    tile.save(ICONS/"icon.png")
    tile.save(ICONS/"icon.icns", format="ICNS")
