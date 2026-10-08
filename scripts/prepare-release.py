import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import zipfile

version = os.environ['GITHUB_REF_NAME'].removeprefix('v')
repository = os.environ['GITHUB_REPOSITORY'].lower()
digest = os.environ['LIYU_IMAGE_DIGEST']
assert re.fullmatch(r'sha256:[0-9a-f]{64}', digest), 'Missing published image digest'
image = f'ghcr.io/{repository}'
output = Path('build/release')
output.mkdir(parents=True, exist_ok=True)
tests = json.loads((output / 'api-tests.json').read_text(encoding='utf-8'))
assert tests['source_revision'] == os.environ['GITHUB_SHA'], 'Wrong test source revision'
assert not tests['working_tree_dirty'], 'Tests must use a clean checkout'
archive = output / f'liyu-server-{version}.zip'
subprocess.run(['git', 'archive', '--format=zip', f'--output={archive}', 'HEAD'], check=True)
record = {'version': version, 'source_revision': os.environ['GITHUB_SHA'],
          'image_tag': f'{image}:v{version}', 'image_pinned': f'{image}@{digest}',
          'platforms': ['linux/amd64', 'linux/arm64']}
(output / 'release.json').write_text(json.dumps(record, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
with zipfile.ZipFile(archive, 'a', compression=zipfile.ZIP_DEFLATED) as package:
    package.writestr('release.json', (output / 'release.json').read_bytes())
checksums = ''.join(f'{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n' for p in sorted(output.iterdir()) if p.is_file() and p.name != 'SHA256SUMS')
(output / 'SHA256SUMS').write_text(checksums, encoding='utf-8')
Path('build/release-notes.md').write_text(
    f'版本镜像（amd64 / arm64）：`{image}:v{version}`\n\n'
    f'固定摘要：`{image}@{digest}`\n\n'
    f'```sh\ndocker pull {image}:v{version}\n```\n\n'
    '附件 ZIP 包含对应版本源码、Compose、Caddy 配置和部署说明；解压后参照 docs/deployment.md 配置本地或线上环境。\n\n'
    '设置 LIYU_IMAGE 为上述标签或固定摘要。api-tests.json 为本次 CI 的测试证据，SHA256SUMS 用于校验附件。\n\n'
    '私有 GHCR 包需要登录；公开部署需将容器包设为 Public。\n', encoding='utf-8')
