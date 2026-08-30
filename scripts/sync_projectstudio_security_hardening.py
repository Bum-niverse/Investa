"""Investa 보안 강화 계획과 검증 상태를 ProjectStudio 기능명세에 멱등 반영한다."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path
from types import ModuleType
from typing import Any


PROJECT_ID = "36e87491-74a8-48ca-a7b8-30fa6ccea131"
SECTION_MARKER = "배포 전 보안 강화 게이트 (2026-08-27)"


def load_api(root: Path) -> ModuleType:
    source = root / "scripts" / "projectstudio_api.py"
    spec = importlib.util.spec_from_file_location("projectstudio_api", source)
    if spec is None or spec.loader is None:
        raise RuntimeError("ProjectStudio 로컬 기획 API를 불러오지 못했습니다.")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def criteria(feature_id: str, values: list[tuple[str, bool]]) -> list[dict[str, Any]]:
    return [
        {"id": f"{feature_id}-ac-{index + 1}", "description": text, "isMet": met, "sortOrder": index}
        for index, (text, met) in enumerate(values)
    ]


def item(feature_id: str, parent_id: str, title: str, description: str, status: str,
         role: str, sort_order: int, checks: list[tuple[str, bool]], color: str = "amber") -> dict[str, Any]:
    return {
        "id": feature_id, "parentId": parent_id, "title": title, "description": description,
        "status": status, "priority": "critical", "role": role, "sortOrder": sort_order,
        "colorKey": color, "acceptanceCriteria": criteria(feature_id, checks),
    }


def desired_features() -> list[dict[str, Any]]:
    root = "feat-security-hardening-20260827"
    return [
        item(root, "req-operations", "배포 전 보안 강화 프로그램",
             "인증·비밀정보·금융 API·Cloud Relay·로컬 데이터·배포 산출물의 공개 배포 차단 조건을 관리한다.",
             "in_progress", "보안 담당 · SRE · 감사로그 담당", 2210, [
                 ("코드에서 자동 검증 가능한 보안 경계를 구현하고 테스트한다.", True),
                 ("Cloud IAM·TTL·키 교체·코드서명을 운영 환경에서 검증한다.", False),
                 ("모든 배포 차단 조건이 해소된 뒤에만 완료 처리한다.", False),
             ]),
        item("feat-security-codex-isolation", root, "Codex 분석 프로세스 격리",
             "Codex가 금융·Cloud 환경변수와 저장소 파일을 상속하지 않고 읽기 전용·네트워크 차단 상태로 분석만 수행한다.",
             "done", "AI 플랫폼 담당 · 보안 담당", 2211, [
                 ("하위 프로세스 환경변수 allowlist가 적용된다.", True),
                 ("빈 임시 작업공간과 명시적 readOnly·networkAccess false 정책을 사용한다.", True),
                 ("승인과 서버 실행 요청이 거부된다.", True),
             ], "green"),
        item("feat-security-github-owner-boundary", root, "GitHub 로컬 작업공간 소유권 경계",
             "GitHub CLI 토큰 노출 없이 숫자 계정 ID를 Windows 자격 증명 관리자에 고정한다.",
             "done", "인증 담당 · 보안 담당", 2212, [
                 ("gh 절대 경로를 직접 실행하고 PowerShell 문자열 실행을 제거한다.", True),
                 ("환경 토큰을 제거하고 사용자 응답에 토큰을 반환하지 않는다.", True),
                 ("다른 GitHub 계정으로 로컬 작업공간에 진입하지 못한다.", True),
             ], "green"),
        item("feat-security-relay-ingress-ttl", root, "Telegram Relay 저장 전 필터·TTL·남용 방지",
             "Cloud에 기록하거나 Telegram으로 전송하기 전에 비밀값을 거부하고 작업 수명과 반복 요청을 제한한다.",
             "done", "Cloud Relay 담당 · 보안 담당", 2213, [
                 ("지시와 결과의 비밀 패턴이 저장·전송 전에 거부된다.", True),
                 ("작업 TTL과 만료 작업 임대 차단이 구현된다.", True),
                 ("rate-limit bucket이 정리되고 상한을 가진다.", True),
             ], "green"),
        item("feat-security-financial-key-policy", root, "금융 API 최소권한 검증",
             "읽기 전용·IP 제한을 검증하고 공급자가 권한 목록을 제공하지 않으면 미검증으로 표시한다.",
             "in_progress", "외부 어댑터 담당 · 보안 담당", 2214, [
                 ("Binance 키 저장·조회 전에 읽기·IP 제한과 위험 권한 비활성화를 검증한다.", True),
                 ("Upbit 권한 검증 불가를 조회 성공과 분리해 표시한다.", True),
                 ("토스·KIS 등 모든 금융 공급자에 동일한 최소권한 증거 계약을 적용한다.", False),
             ]),
        item("feat-security-csp-release-audit", root, "CSP·릴리스 비밀 검사",
             "개발·운영 출처를 분리하고 Git 추적 파일·산출물의 비밀정보를 배포 전에 차단한다.",
             "in_progress", "프론트엔드 보안 담당 · 릴리스 담당", 2215, [
                 ("프로덕션 CSP에서 개발 localhost·WebSocket 출처가 제거된다.", True),
                 ("로컬 비밀 스캔과 금지 산출물 검사가 제공된다.", True),
                ("GitHub Actions에 Gitleaks·전체 검증·RustSec 게이트가 정의된다.", True),
                ("원격 Actions 왕복과 Windows 설치파일 서명·provenance를 검증한다.", False),
             ]),
        item("feat-security-dependency-advisories", root, "Rust 의존성 경고 해소·플랫폼 검증",
             "RustSec가 보고한 유지중단·unsound 전이 의존성을 Tauri 호환 범위에서 해소하고 Windows·macOS·Linux 영향을 구분한다.",
             "in_progress", "데스크톱 플랫폼 담당 · 보안 담당", 2216, [
                 ("cargo audit를 실행해 18개 허용 경고와 직접·전이 의존성 경로를 확인한다.", True),
                ("event-listener·glib unsound 경고의 실제 사용 경로와 플랫폼 영향을 확인한다.", True),
                 ("Tauri·플러그인 호환 업데이트 후 Windows·macOS·Linux 빌드와 회귀 검사를 통과한다.", False),
                 ("릴리스 CI에서 새 RustSec 경고를 실패 또는 승인 기록으로 관리한다.", False),
             ]),
        item("feat-security-cloud-operations", root, "Cloud IAM·Firestore·키 교체 운영 검증",
             "Cloud Run 호출 경계, 최소 IAM, Firestore TTL, 로그 마스킹과 노출 의심 키 교체를 운영 환경에서 확인한다.",
             "planned", "Cloud 관리자 · 보안 담당", 2217, [
                ("Cloud Run·IAM·Firestore TTL·Secret 목록을 값 노출 없이 조회하는 감사 스크립트가 있다.", True),
                ("Cloud Run 인증·webhook 공개 경계가 실제 프로젝트에서 검증된다.", False),
                 ("Firestore TTL 정책과 최소 IAM이 실제 프로젝트에 적용된다.", False),
                 ("노출 의심 키를 폐기·재발급하고 로그 보존·마스킹을 확인한다.", False),
             ]),
        item("feat-security-local-data-signing", root, "로컬 DB 보호·설치파일 코드서명",
             "SQLite의 Windows 계정 경계와 Codex sandbox 차단을 유지하고 Windows 설치·업데이트 서명을 완성한다. Apple 배포는 사용자 검증 이후 별도 범위다.",
             "in_progress", "데스크톱 플랫폼 담당 · 보안 담당", 2218, [
                 ("앱 데이터 폴더에 CodexSandboxUsers 명시적 거부 ACL을 적용하고 시작 시 재확인한다.", True),
                 ("로컬 DB 암호화 여부와 백업·삭제 정책을 확정한다.", False),
                 ("Windows 코드서명을 적용한다.", False),
                 ("업데이트 manifest와 패키지 무결성을 검증한다.", False),
             ]),
        item("feat-security-social-login", root, "Google·Apple 선택 로그인 보안 경계",
             "최초 검증 계정을 통합 로컬 작업공간 소유자로 고정하고 이후 GitHub·Google 계정은 소유자 세션에서 명시적으로 연결한다. Apple은 공식 HTTPS callback 준비 전 비활성으로 유지한다.",
             "in_progress", "인증 담당 · 보안 담당", 2219, [
                 ("Google PKCE·state·127.0.0.1 임시 callback과 최소 scope를 구현한다.", True),
                 ("공급자 access/refresh token 없이 통합 소유자와 연결 계정의 불변 ID만 Windows 보안 저장소에 고정한다.", True),
                 ("미연결 계정 진입을 거부하고 소유자 세션에서만 GitHub·Google 계정을 연결한다.", True),
                 ("Google Desktop OAuth Client ID·Secret으로 실제 로그인 왕복을 검증한다.", True),
                 ("기존 GitHub 소유자에서 미연결 Google 거부·연결·재로그인 UI 왕복을 검증한다.", False),
                 ("Apple은 Services ID·검증 도메인·HTTPS callback 전까지 안전하게 비활성으로 표시한다.", True),
                 ("Apple 로그인 서버 검증은 Apple Developer 설정 준비 후 별도 구현·검증한다.", False),
             ]),
    ]


def normalize_flow(current: dict[str, Any]) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    nodes = []
    for node in current["userFlow"]["nodes"]:
        metadata = json.loads(node.get("metadata_json") or "{}")
        nodes.append({
            "id": node["id"], "laneId": node["lane_id"], "title": node["title"], "description": node["description"],
            "kind": node["kind"], "positionX": node["position_x"], "positionY": node["position_y"],
            "colorKey": node.get("color_key") or "violet", "depth": node.get("depth"), "parentId": node.get("parent_id"),
            "linkedFeatureIds": json.loads(node.get("linked_feature_ids") or "[]"), "branchCondition": node.get("branch_condition"),
            "inputArtifacts": metadata.get("inputArtifacts", []), "outputArtifacts": metadata.get("outputArtifacts", []),
            "methods": metadata.get("methods", []), "validation": metadata.get("validation", ""),
            "failureHandling": metadata.get("failureHandling", ""), "codePaths": metadata.get("codePaths", []),
            "testPaths": metadata.get("testPaths", []), "completionCriteria": metadata.get("completionCriteria", ""),
            "isCompleted": bool(metadata.get("isCompleted", False)),
        })
    edges = [{"id": edge["id"], "sourceNodeId": edge["source_node_id"], "targetNodeId": edge["target_node_id"]} for edge in current["userFlow"]["edges"]]
    return nodes, edges


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("projectstudio_root", type=Path)
    args = parser.parse_args()
    api = load_api(args.projectstudio_root)
    database = api.default_database_path()
    with api.connect_database(database, writable=True) as connection:
        current = api.get_project(connection, PROJECT_ID)
        desired = desired_features()
        replacements = {feature["id"]: feature for feature in desired}
        existing = {feature["id"]: feature for feature in current["features"]}
        keys = ("parentId", "title", "description", "status", "priority", "role", "sortOrder", "colorKey", "acceptanceCriteria")
        changed = any(feature["id"] not in existing or any(existing[feature["id"]].get(key) != feature.get(key) for key in keys) for feature in desired)
        merged = [replacements.pop(feature["id"], feature) for feature in current["features"]]
        merged.extend(replacements.values())
        markdown = current["project"]["prd_markdown"]
        section_missing = SECTION_MARKER not in markdown
        if section_missing:
            markdown += f"""

## {SECTION_MARKER}
- 공개 배포 전 인증·비밀정보·금융 API·Cloud Relay·로컬 데이터·설치파일을 하나의 보안 게이트로 관리한다.
- Codex와 외부 AI는 분석만 수행하며 주문·출금·자격정보·위험정책 변경 권한을 받지 않는다.
- Telegram 지시와 결과는 Cloud 저장 전 비밀 패턴을 거부하고 TTL·서명·replay·allowlist를 적용한다.
- 금융 API는 읽기 전용·IP 제한의 검증 증거가 있어야 연결로 표시하며 검증 불가 상태를 성공으로 꾸미지 않는다.
- Cloud IAM·Firestore TTL·키 교체·코드서명은 실제 운영 검증 전까지 완료 처리하지 않는다.
- 최초 검증 로그인 계정은 통합 로컬 작업공간 소유자가 되며 이후 계정은 소유자 세션에서 명시적으로 연결한다. 로그인 공급자는 금융 자격정보와 분리하고 Apple은 공식 HTTPS callback 준비 전 비활성으로 둔다.
- Apple 앱 배포·Developer ID·notarization은 사용자 실사용 검증 이후 별도 범위로 연기한다.
- 상세 위협 모델과 배포 차단 조건은 `docs/security-hardening-2026-08-27.md`를 따른다.
"""
        if not changed and not section_missing:
            print(json.dumps({"projectId": PROJECT_ID, "committed": False, "message": "동일한 보안 기획이 이미 반영되어 있습니다."}, ensure_ascii=False, indent=2))
            return
        nodes, edges = normalize_flow(current)
        bundle = {
            "schemaVersion": 1, "projectId": PROJECT_ID,
            "expectedPrdRevisionNumber": current["project"]["revision_number"],
            "prd": {"title": current["project"]["prd_title"], "markdown": markdown},
            "features": merged, "userFlow": {"nodes": nodes, "edges": edges},
        }
        api.validate_bundle(bundle)
        print(json.dumps(api.apply_bundle(connection, database, bundle, commit=True), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
