"""Assemble the redistributable Beside source, excluding notebooks and build state."""
from pathlib import Path
import subprocess
from zipfile import ZIP_DEFLATED, ZipFile

root = Path(__file__).resolve().parents[1]
files = [
    "README.md", "CONTRIBUTING.md", "SECURITY.md", "LICENSE", ".gitignore", "index.html", "package.json",
    "package-lock.json", "tsconfig.json", "playwright.config.ts", "rust-toolchain.toml",
    "src-tauri/Cargo.toml", "src-tauri/Cargo.lock", "src-tauri/build.rs",
    "src-tauri/runtime/LICENSE", "src-tauri/overlay.m", "src-tauri/calendar.m", "src-tauri/native.m", "src-tauri/Info.plist", "src-tauri/entitlements.plist", "src-tauri/meeting.m", "src-tauri/voice.m", "src-tauri/activity.m", "src-tauri/bag_vision.m", "src-tauri/tauri.conf.json", "website/index.html",
    "website/Newsreader.ttf", "website/paper.png", "website/beside.png", "website/Geist.ttf", "website/film.js",
]
folders = [
    "website/media", ".github/workflows", "catalog", "src", "public", "scripts", "tests", "prompts", "licenses", "docs",
    "src-tauri/confidential", "src-tauri/src", "src-tauri/data", "src-tauri/capabilities", "src-tauri/icons", "examples/writing",
]
# Only ship reviewed files. A source distribution carries the same inventory so
# it can be repackaged without Git metadata.
manifest = "SOURCE_FILES"
if (root / ".git").exists():
    raw = subprocess.check_output(["git", "ls-files", "-z"], cwd=root)
    inventory = [name.decode() for name in raw.split(b"\0") if name]
elif (root / manifest).is_file():
    inventory = (root / manifest).read_text().splitlines()
else:
    raise SystemExit("Package from a Git checkout or an extracted Beside source archive.")
paths = sorted({
    name for name in inventory
    if (name in files or any(name.startswith(folder + "/") for folder in folders))
    and not Path(name).is_absolute() and ".." not in Path(name).parts
})
missing = set(files) - set(paths)
if missing:
    raise SystemExit("Missing required source files: " + ", ".join(sorted(missing)))
target = root / "release/Beside-source.zip"
target.parent.mkdir(exist_ok=True)
with ZipFile(target, "w", compression=ZIP_DEFLATED) as archive:
    for name in paths:
        p = root / name
        if not p.resolve().is_relative_to(root) or not p.is_file():
            raise SystemExit(f"Invalid source file: {name}")
        archive.write(p, Path("Beside") / name)
    archive.writestr("Beside/" + manifest, "\n".join(paths) + "\n")
with ZipFile(target) as archive:
    if archive.testzip():
        raise SystemExit("Source archive integrity check failed")
print(target)
