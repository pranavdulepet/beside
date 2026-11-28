"""Collect dependency notices for the Mac distribution from the locked build."""
import json
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

root = Path(__file__).resolve().parents[1]
metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--locked", "--offline", "--filter-platform", "aarch64-apple-darwin",
    "--format-version", "1", "--manifest-path", str(root / "src-tauri/Cargo.toml"),
]))
index = []
texts = defaultdict(list)

def include(package, folder, extra=()):
    names = ("license", "licence", "copying", "notice", "copyright", "unlicense")
    paths = [p for p in folder.iterdir() if p.name.lower().startswith(names) and p.is_file()]
    paths.extend(folder / p for p in extra)
    for p in sorted(set(paths)):
        texts[p.read_text(errors="replace").strip()].append(f"{package} / {p.relative_to(folder)}")

for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
    name, version = package["name"], package["version"]
    if name == "beside":
        continue
    label = f"{name} {version}"
    index.append(f"{label}  {package['license'] or package['license_file']}  https://crates.io/crates/{name}/{version}")
    extra = {"sqlcipher-src": ["sqlcipher/LICENSE-sqlcipher", "sqlcipher/LICENSE-libtomcrypt"], "openssl-src": ["openssl/LICENSE.txt"]}.get(name, [])
    if package.get("license_file"):
        extra = [*extra, package["license_file"]]
    include(label, Path(package["manifest_path"]).parent, extra)
# Include the modules actually linked into the confidential helper.
helper = root / "src-tauri/confidential"
modules = subprocess.check_output(
    ["go", "list", "-mod=readonly", "-deps", "-f", "{{with .Module}}{{if not .Main}}{{.Path}} {{.Version}} {{.Dir}}{{end}}{{end}}", "."],
    cwd=helper, text=True)
for module in sorted(set(modules.splitlines()) - {""}):
    name, version, directory = module.split(" ", 2)
    label = f"{name} {version}"
    index.append(f"{label}  https://pkg.go.dev/{name}@{version}")
    include(label, Path(directory))
goroot = Path(subprocess.check_output(["go", "env", "GOROOT"], cwd=helper, text=True).strip())
include("Go runtime", goroot)

lock = json.loads((root / "package-lock.json").read_text())
for path, record in sorted(lock["packages"].items()):
    if not path or record.get("dev") or record.get("optional"):
        continue
    name = path.rsplit("node_modules/", 1)[-1]
    folder = root / path
    package = json.loads((folder / "package.json").read_text())
    label = f"{name} {package['version']}"
    index.append(f"{label}  {package.get('license', '')}  https://www.npmjs.com/package/{name}/v/{package['version']}")
    include(label, folder)

parts = ["Beside dependency notices", "\n".join(index)]
for content, labels in texts.items():
    parts.extend(["\n".join(labels), content])
Path(sys.argv[1]).write_text("\n\n".join(parts) + "\n")
