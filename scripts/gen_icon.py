"""Generate the ShuttleSFTP app icon: white space shuttle on blue rounded background."""
from PIL import Image, ImageDraw

S = 1024
SS = 4  # supersampling factor
W = S * SS

BLUE = (37, 99, 235, 255)
WHITE = (255, 255, 255, 255)

img = Image.new("RGBA", (W, W), (0, 0, 0, 0))
d = ImageDraw.Draw(img)

# Blue rounded-square background
radius = int(W * 0.22)
d.rounded_rectangle([0, 0, W - 1, W - 1], radius=radius, fill=BLUE)

# Subtle vertical highlight gradient, clipped to the rounded square
grad = Image.new("RGBA", (W, W), (0, 0, 0, 0))
gd = ImageDraw.Draw(grad)
for y in range(W):
    a = int(50 * (1 - y / W))
    gd.line([(0, y), (W, y)], fill=(255, 255, 255, a))
mask = Image.new("L", (W, W), 0)
ImageDraw.Draw(mask).rounded_rectangle([0, 0, W - 1, W - 1], radius=radius, fill=255)
img = Image.composite(Image.alpha_composite(img, grad), img, mask)
d = ImageDraw.Draw(img)


def X(v):
    return v / 100 * W


def Y(v):
    return v / 100 * W


def P(x, y):
    return (X(x), Y(y))


# --- Space shuttle (orbiter, top view, nose up), centered ---
fuselage = [
    P(50, 11),
    P(54.5, 17),
    P(56.5, 24),
    P(56.5, 76),
    P(54, 84),
    P(46, 84),
    P(43.5, 76),
    P(43.5, 24),
    P(45.5, 17),
]
d.polygon(fuselage, fill=WHITE)

# Delta wings
d.polygon([P(43.5, 38), P(43.5, 76), P(20, 76), P(22, 68)], fill=WHITE)
d.polygon([P(56.5, 38), P(56.5, 76), P(80, 76), P(78, 68)], fill=WHITE)

# Wing trailing edges (swept back)
d.polygon([P(20, 76), P(26, 80), P(43.5, 80), P(43.5, 76)], fill=WHITE)
d.polygon([P(80, 76), P(74, 80), P(56.5, 80), P(56.5, 76)], fill=WHITE)

# Tail fin (top view: short stripe past the tail)
d.polygon([P(47.5, 80), P(52.5, 80), P(51.5, 90), P(48.5, 90)], fill=WHITE)

# Engine nozzles
d.ellipse([X(43.8), Y(83), X(48.0), Y(88)], fill=WHITE)
d.ellipse([X(52.0), Y(83), X(56.2), Y(88)], fill=WHITE)

# Cockpit window (blue cutout near nose)
d.rounded_rectangle([X(46.5), Y(17.5), X(53.5), Y(23)], radius=int(W * 0.012), fill=BLUE)

# Payload bay detail (blue slot along mid-fuselage)
d.rounded_rectangle([X(47), Y(30), X(53), Y(66)], radius=int(W * 0.01), fill=BLUE)

img = img.resize((S, S), Image.LANCZOS)
img.save(r"D:\workspace\ShuttleSFTP\scripts\app-icon.png")
print("saved app-icon.png")
