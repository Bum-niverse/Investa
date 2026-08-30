"""외부 데이터·AI 공급자 선정과 구현 상태를 ProjectStudio 기능명세에 멱등 반영한다."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path
from types import ModuleType
from typing import Any


PROJECT_ID = "36e87491-74a8-48ca-a7b8-30fa6ccea131"
SECTION_MARKER = "외부 데이터·AI 공급자 결정 (2026-08-27)"
AUTO_REFRESH_MARKER = "로그인 후 읽기 전용 전체 연결 자동 조회"


def load_api(root: Path) -> ModuleType:
    source = root / "scripts" / "projectstudio_api.py"
    spec = importlib.util.spec_from_file_location("projectstudio_api", source)
    if spec is None or spec.loader is None:
        raise RuntimeError("ProjectStudio 로컬 기획 API를 불러오지 못했습니다.")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def acceptance(feature_id: str, items: list[tuple[str, bool]]) -> list[dict[str, Any]]:
    return [
        {"id": f"{feature_id}-ac-{index + 1}", "description": description, "isMet": met, "sortOrder": index}
        for index, (description, met) in enumerate(items)
    ]


def features() -> list[dict[str, Any]]:
    return [
        {
            "id": "feat-official-provider-selection-20260827",
            "parentId": "req-broker",
            "title": "공식 시장·뉴스·커뮤니티 공급자 선정",
            "description": "자산군별 공식 공급자, 라이선스, 비용, 지연 상태와 연결 우선순위를 확정하고 미연결 값을 꾸며내지 않는다.",
            "status": "done",
            "priority": "high",
            "role": "외부 어댑터 담당 · 시장데이터 엔지니어 · 보안 담당",
            "sortOrder": 2180,
            "colorKey": "green",
            "acceptanceCriteria": acceptance("feat-official-provider-selection-20260827", [
                ("국내·미국 주식과 국내 지수는 토스, 코인은 Upbit·Binance 공식 API를 우선 사용한다.", True),
                ("미국 공시는 SEC, 국내 공시는 OpenDART, 국내 일반 뉴스는 네이버 뉴스 검색 API를 선정한다.", True),
                ("NASDAQ 공식 지수는 Nasdaq Data Link/GIDS 라이선스 승인 전 미연결로 유지한다.", True),
                ("커뮤니티는 Telegram을 기본으로 하고 Reddit·Stocktwits는 공식 개발자 승인 후 선택 연결한다.", True),
                ("KIS는 현재 연결 범위에서 제외하고 기존 보류 상태를 유지한다.", True),
                ("설정의 상단 연결 요약과 각 접힘 항목에 체크·주황·회색 상태 및 텍스트 범례를 표시한다.", True),
                ("Telegram은 백엔드의 세션 인증 상태와 선택 채널 수를 상단 요약과 접힘 제목에 동기화한다.", True),
            ]),
        },
        {
            "id": "feat-ai-provider-analysis-adapters",
            "parentId": "feat-official-provider-selection-20260827",
            "title": "Claude·Antigravity 분석 공급자 어댑터",
            "description": "Codex 외 AI를 공통 분석 전용 계약으로 연결하고 비밀정보·주문·위험정책 권한을 격리한다.",
            "status": "in_progress",
            "priority": "high",
            "role": "AI 플랫폼 담당 · 보안 담당",
            "sortOrder": 2181,
            "colorKey": "violet",
            "acceptanceCriteria": acceptance("feat-ai-provider-analysis-adapters", [
                ("Claude Messages API와 Antigravity Interactions API의 직접 REST 어댑터를 구현한다.", True),
                ("API 키와 모델 설정을 Windows 자격 증명 관리자에만 저장하고 키 저장 시 외부 호출을 만들지 않는다.", True),
                ("Antigravity는 검색·URL 읽기만 허용하고 코드 실행·파일·custom function·MCP를 제공하지 않는다.", True),
                ("분석 응답은 공급자·모델·관측시각·토큰 사용량·분석 전용 상태로 정규화한다.", True),
                ("사용자 API 키로 Claude·Antigravity 실제 왕복을 검증한다.", False),
                ("44인 직원의 RoleReport·DepartmentReport·취소·스트리밍 계약을 공급자 공통 계층에 연결한다.", False),
            ]),
        },
        {
            "id": "feat-automatic-connection-refresh",
            "parentId": "feat-official-provider-selection-20260827",
            "title": "로그인 후 전체 연결 자동 조회",
            "description": "GitHub 로그인 완료 뒤 저장된 공급자의 읽기 전용 연결 상태를 한 번 자동 확인하고 설정에서 전체 연결 조회를 다시 실행한다.",
            "status": "done",
            "priority": "high",
            "role": "외부 어댑터 담당 · 보안 담당 · UI 담당",
            "sortOrder": 2184,
            "colorKey": "cyan",
            "acceptanceCriteria": acceptance("feat-automatic-connection-refresh", [
                ("GitHub 로그인 완료 뒤 설정을 열지 않아도 읽기 전용 전체 연결 조회를 한 번 실행한다.", True),
                ("설정의 전체 연결 조회 버튼으로 같은 검사를 중복 실행 없이 다시 요청할 수 있다.", True),
                ("토스·Upbit·Binance·KIS·SEC는 저장 상태와 실제 읽기 전용 조회 상태를 구분한다.", True),
                ("Telegram·Codex는 세션 상태를 확인하고 외부 AI는 유료 분석 호출 없이 설정 상태만 확인한다.", True),
                ("조회 결과에 연결·확인 필요·미연결·실패 수와 완료 시각을 표시한다.", True),
                ("전체 조회는 주문·출금·위험정책 변경을 호출하지 않는다.", True),
            ]),
        },
        {
            "id": "feat-official-realtime-stream-adapters",
            "parentId": "feat-official-provider-selection-20260827",
            "title": "자산군별 공식 실시간 스트림",
            "description": "기존 REST snapshot 위에 토스·Upbit·Binance 공식 WebSocket을 연결하고 sequence gap·stale·재연결·재시작 대사를 검증한다.",
            "status": "in_progress",
            "priority": "high",
            "role": "시장데이터 엔지니어",
            "sortOrder": 2182,
            "colorKey": "cyan",
            "acceptanceCriteria": acceptance("feat-official-realtime-stream-adapters", [
                ("토스·Upbit·Binance REST snapshot과 PIT 정규화 계약을 재사용한다.", True),
                ("Upbit 현물과 Binance 현물·USDⓈ-M·COIN-M 공식 WebSocket을 구현한다.", True),
                ("토스 체결·호가 구독 선언과 ack·부분 거부·오류·시장 메시지 파서를 구현하고 개인 주문 채널을 차단한다.", True),
                ("토스 인증 WebSocket Rust 전송에서 토큰 비노출·시장 topic 전용·PING·ack timeout·jitter 재연결을 구현한다.", True),
                ("토스 국장·미장 장중 체결·호가, PING/pong과 장시간 재연결을 실제 왕복 검증한다.", False),
                ("sequence gap, stale 관측, rate limit, 재연결과 앱 재시작 대사를 자동 검사한다.", False),
                ("NASDAQ는 공식 라이선스 전 proxy 숫자나 임의 지수를 표시하지 않는다.", True),
            ]),
        },
        {
            "id": "feat-official-news-community-adapters",
            "parentId": "feat-official-provider-selection-20260827",
            "title": "OpenDART·네이버 뉴스·공식 커뮤니티 어댑터",
            "description": "공식 공시·뉴스·커뮤니티 API의 출처·게시·관측시각·수정·결측을 정규화하고 외부 본문을 명령으로 실행하지 않는다.",
            "status": "in_progress",
            "priority": "high",
            "role": "뉴스·소셜 분석 담당 · 외부 어댑터 담당",
            "sortOrder": 2183,
            "colorKey": "amber",
            "acceptanceCriteria": acceptance("feat-official-news-community-adapters", [
                ("SEC Company Facts·Submissions 읽기 전용 어댑터를 구현한다.", True),
                ("Telegram 선택 방송 채널 읽기 전용 수집과 리비전 보존을 구현한다.", True),
                ("사용자 연락처·MTProto 자격정보로 SEC·Telegram 실제 왕복을 검증한다.", False),
                ("OpenDART와 네이버 뉴스 검색 공식 API 어댑터를 구현한다.", False),
                ("Reddit·Stocktwits는 개발자 승인·약관·보존정책 확정 후에만 구현한다.", False),
            ]),
        },
    ]


def normalize_flow(current: dict[str, Any]) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    nodes = []
    for node in current["userFlow"]["nodes"]:
        metadata = json.loads(node.get("metadata_json") or "{}")
        nodes.append({
            "id": node["id"], "laneId": node["lane_id"], "title": node["title"], "description": node["description"], "kind": node["kind"],
            "positionX": node["position_x"], "positionY": node["position_y"], "colorKey": node.get("color_key") or "violet", "depth": node.get("depth"),
            "parentId": node.get("parent_id"), "linkedFeatureIds": json.loads(node.get("linked_feature_ids") or "[]"), "branchCondition": node.get("branch_condition"),
            "inputArtifacts": metadata.get("inputArtifacts", []), "outputArtifacts": metadata.get("outputArtifacts", []), "methods": metadata.get("methods", []),
            "validation": metadata.get("validation", ""), "failureHandling": metadata.get("failureHandling", ""), "codePaths": metadata.get("codePaths", []),
            "testPaths": metadata.get("testPaths", []), "completionCriteria": metadata.get("completionCriteria", ""), "isCompleted": bool(metadata.get("isCompleted", False)),
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
        desired = features()
        existing = {item["id"]: item for item in current["features"]}
        for item in desired:
            if item["id"] in existing:
                item["sortOrder"] = existing[item["id"]]["sortOrder"]
        replacements = {item["id"]: item for item in desired}
        keys = ("parentId", "title", "description", "status", "priority", "role", "sortOrder", "colorKey", "acceptanceCriteria")
        changed = any(item["id"] not in existing or any(existing[item["id"]].get(key) != item.get(key) for key in keys) for item in desired)
        merged = [replacements.pop(item["id"], item) for item in current["features"]]
        merged.extend(replacements.values())
        markdown = current["project"]["prd_markdown"]
        section_missing = SECTION_MARKER not in markdown
        if section_missing:
            markdown += f"""

## {SECTION_MARKER}
- KIS는 현재 연결 범위에서 제외하고 기존 보류 코드를 유지한다. 실주문·출금은 계속 잠근다.
- 국장·미장 종목은 토스, 코인은 Upbit·Binance, 미국 공시는 SEC, 국내 공시는 OpenDART, 국내 일반 뉴스는 네이버 뉴스 검색 API를 우선한다.
- NASDAQ 공식 지수는 Nasdaq Data Link/GIDS의 라이선스·비용을 승인하기 전까지 미연결로 표시한다.
- 커뮤니티는 사용자가 선택한 Telegram 채널을 기본으로 하고 Reddit·Stocktwits는 공식 개발자 승인 후 선택 연결한다.
- Claude와 Google Antigravity는 분석 전용 REST 어댑터를 사용한다. API 키는 Windows 보안 저장소에만 두고 AI에 계좌·주문·출금·위험정책 도구를 주지 않는다.
- Antigravity에는 검색·URL 읽기만 제공하며 코드 실행·원격 파일·custom function·MCP를 제공하지 않는다.
"""
        auto_refresh_missing = AUTO_REFRESH_MARKER not in markdown
        if auto_refresh_missing:
            markdown += f"\n- {AUTO_REFRESH_MARKER}: 설정을 열기 전에도 로그인 직후 한 번 실행하며 주문·출금·유료 AI 분석은 호출하지 않는다. 설정의 전체 연결 조회로 수동 재검사할 수 있다.\n"
        if not changed and not section_missing and not auto_refresh_missing:
            print(json.dumps({"projectId": PROJECT_ID, "committed": False, "message": "동일한 공급자 기획이 이미 반영되어 있습니다."}, ensure_ascii=False, indent=2))
            return
        nodes, edges = normalize_flow(current)
        bundle = {
            "schemaVersion": 1,
            "projectId": PROJECT_ID,
            "expectedPrdRevisionNumber": current["project"]["revision_number"],
            "prd": {"title": current["project"]["prd_title"], "markdown": markdown},
            "features": merged,
            "userFlow": {"nodes": nodes, "edges": edges},
        }
        api.validate_bundle(bundle)
        print(json.dumps(api.apply_bundle(connection, database, bundle, commit=True), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
