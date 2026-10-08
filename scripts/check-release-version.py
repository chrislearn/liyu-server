import os
import re
import tomllib
from pathlib import Path

version = tomllib.loads(Path('Cargo.toml').read_text(encoding='utf-8'))['package']['version']
tag = os.environ['GITHUB_REF_NAME']
assert re.fullmatch(r'v\d+\.\d+\.\d+', tag), 'Release tag must be vMAJOR.MINOR.PATCH'
assert tag == 'v' + version, f'Tag {tag} does not match Cargo version {version}'
