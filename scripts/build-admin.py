"""Build the Dioxus Web UI and install its complete public output for Salvo."""
from pathlib import Path
import hashlib
import shutil
import subprocess
root=Path(__file__).resolve().parents[1]
subprocess.run(['dx','build','--web','--release','--locked','--debug-symbols','false'],cwd=root/'admin-ui',check=True)
source=root/'admin-ui/target/dx/liyu-admin/release/web/public'
if not (source/'index.html').is_file(): raise SystemExit('Dioxus index.html missing')
# Replace only generated output, never runtime media.
stage=root/'web/dist-next'
if stage.exists(): shutil.rmtree(stage)
shutil.copytree(source,stage)
# Publish CSS with the WASM bundle, so a frontend-only update never uses
# an older stylesheet embedded in the running Rust server.
css=(root/'web/admin.css').read_bytes()
css_name=f'admin-{hashlib.sha256(css).hexdigest()[:16]}.css'
(stage/'assets').mkdir(exist_ok=True)
(stage/'assets'/css_name).write_bytes(css)
index=stage/'index.html'
html=index.read_text()
stylesheet='href="/admin/app.css"'
if stylesheet not in html: raise SystemExit('Dioxus stylesheet link missing')
index.write_text(html.replace(stylesheet, f'href="/admin/assets/{css_name}"'))
target=root/'web/dist'
if target.exists(): shutil.rmtree(target)
stage.rename(target)
print('Dioxus UI installed in web/dist; restart server only if Rust code changed.')
