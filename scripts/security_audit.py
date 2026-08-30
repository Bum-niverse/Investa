"""배포 전 Git 추적 파일의 비밀정보와 금지 산출물을 실패 우선으로 검사한다."""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
FORBIDDEN_SUFFIXES = {".pfx", ".p12", ".pem", ".key", ".sqlite", ".sqlite3", ".db"}
FORBIDDEN_NAMES = {".env", "service-account.json", "credentials.json"}
PATTERNS = {
    "private-key": re.compile(rb"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"),
    "github-token": re.compile(rb"\b(?:ghp|github_pat)_[A-Za-z0-9_]{20,}\b"),
    "openai-token": re.compile(rb"\bsk-(?:proj-)?[A-Za-z0-9_-]{20,}\b"),
    "google-api-key": re.compile(rb"\bAIza[A-Za-z0-9_-]{20,}\b"),
    "slack-token": re.compile(rb"\bxox[baprs]-[A-Za-z0-9-]{12,}\b"),
}


def git(*args: str) -> bytes:
    result = subprocess.run(["git", *args], cwd=ROOT, check=True, capture_output=True)
    return result.stdout


def main() -> int:
    tracked = [Path(line) for line in git("ls-files", "--cached", "--others", "--exclude-standard", "-z").decode("utf-8").split("\0") if line]
    failures: list[str] = []
    for relative in tracked:
        lowered = relative.name.lower()
        if lowered in FORBIDDEN_NAMES or relative.suffix.lower() in FORBIDDEN_SUFFIXES:
            failures.append(f"금지 산출물 추적: {relative.as_posix()}")
            continue
        path = ROOT / relative
        if not path.is_file() or path.stat().st_size > 5_000_000:
            continue
        data = path.read_bytes()
        for label, pattern in PATTERNS.items():
            if pattern.search(data):
                failures.append(f"비밀 패턴({label}) 발견: {relative.as_posix()}")
    history = git("log", "-p", "--all", "--", ".")
    for label, pattern in PATTERNS.items():
        if pattern.search(history):
            failures.append(f"Git 전체 이력에서 비밀 패턴({label}) 발견 · 값은 출력하지 않음")
    if failures:
        print("보안 검사 실패:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print(f"보안 검사 통과: Git 추적·추가 예정 파일 {len(tracked)}개, 금지 산출물·고신뢰 비밀 패턴 없음")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
