"""Render Beside's ink signature for the sheet, menu bar and Mac app."""
from pathlib import Path
from PIL import Image, ImageDraw

PAPER = '#E5E99C'
INK = '#51233F'


def mark():
    import json
    points = json.loads(Path('src/signature.json').read_text())
    points = [(14 + 231*x, 45 + 161*y) for x, y in points]
    image = Image.new('RGBA', (256, 256))
    pen = ImageDraw.Draw(image)
    start = points[0]
    for i, control in enumerate(points[:-1]):
        nxt = points[i+1]
        end = tuple((a+b)/2 for a,b in zip(control,nxt))
        for j in range(12):
            t = j/12
            x,y = ((1-t)**2*a+2*(1-t)*t*b+t*t*c for a,b,c in zip(start,control,end))
            pressure = min(1,(i+t+.2)/2,(len(points)-1-i-t)/3)
            r = (2.4 + 3.1 * (end[1] > start[1])) * pressure + .9
            pen.ellipse((x-r,y-r,x+r,y+r),fill=INK)
        start = end
    pen.line([start,points[-1]],fill=INK,width=2)
    return image


if __name__ == '__main__':
    app = Path('src-tauri/icons')
    source = mark()
    source.save('public/materials/beside.png')
    small = source.crop((8, 12, 251, 255)).resize((36, 36), Image.Resampling.LANCZOS)
    small.save(app / 'tray.png')
    icon = Image.new('RGBA', (1024, 1024))
    ImageDraw.Draw(icon).rounded_rectangle((48, 48, 976, 976), radius=204, fill=PAPER)
    icon.alpha_composite(source.resize((816, 816), Image.Resampling.LANCZOS), (100, 106))
    icon.save(app / 'icon.png')
    icon.save(app / 'icon.icns', format='ICNS')
