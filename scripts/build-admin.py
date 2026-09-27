"""Build the Dioxus Web UI and install its complete public output for Salvo."""
from pathlib import Path
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
target=root/'web/dist'
if target.exists(): shutil.rmtree(target)
stage.rename(target)
print('Dioxus UI installed in web/dist; restart server only if Rust code changed.')
