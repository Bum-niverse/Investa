"""Investa 구현 증거와 ProjectStudio 기능 상태를 보수적으로 정합화한다."""

from __future__ import annotations

import argparse
import copy
import importlib.util
import json
import re
from pathlib import Path
from types import ModuleType
from typing import Any


PROJECT_ID = "36e87491-74a8-48ca-a7b8-30fa6ccea131"
AUDIT_SECTION_TITLE = "ProjectStudio 구현 상태 정합성 감사"


def load_api(projectstudio_root: Path) -> ModuleType:
    source = projectstudio_root / "scripts" / "projectstudio_api.py"
    spec = importlib.util.spec_from_file_location("projectstudio_api", source)
    if spec is None or spec.loader is None:
        raise RuntimeError("ProjectStudio 로컬 기획 API를 불러오지 못했습니다.")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def criterion(feature_id: str, index: int, description: str, met: bool) -> dict[str, Any]:
    return {
        "id": f"{feature_id}-ac-{index + 1}",
        "description": description,
        "isMet": met,
        "sortOrder": index,
    }


def feature(
    feature_id: str,
    parent_id: str,
    title: str,
    description: str,
    status: str,
    priority: str,
    role: str,
    checks: list[tuple[str, bool]],
    color_key: str,
) -> dict[str, Any]:
    return {
        "id": feature_id,
        "parentId": parent_id,
        "title": title,
        "description": description,
        "status": status,
        "priority": priority,
        "role": role,
        "sortOrder": 0,
        "colorKey": color_key,
        "acceptanceCriteria": [
            criterion(feature_id, index, description_text, met)
            for index, (description_text, met) in enumerate(checks)
        ],
    }


def new_features() -> list[dict[str, Any]]:
    return [
        feature(
            "feat-strategy-protection-operations-visibility",
            "req-risk",
            "전략 보호 판정·잠금 운영 화면",
            "저장된 전략 보호 판정 이력을 매매운영 화면에서 조회하고 잠금 범위·사유·만료를 설명한다.",
            "in_progress",
            "high",
            "리스크관리 총괄 · 매매운영 담당",
            [
                ("SQLite 판정 이력 조회 명령과 불변 저장이 구현되어 있다.", True),
                ("운영 화면에서 허용·차단 사유와 전역·종목 잠금 만료 시각을 확인한다.", False),
                ("잠금 시작·만료·해제 알림을 중복 없이 기록하고 매도·취소는 계속 허용한다.", False),
            ],
            "amber",
        ),
        feature(
            "feat-portfolio-risk-ledger-composition-ui",
            "req-performance",
            "원장 기반 포트폴리오 위험 구성·화면",
            "실제 내부 모의원장 포지션과 시점 정합 가격 수익률을 통화별로 구성해 저장된 VaR·CVaR·상관·스트레스 엔진에 전달한다.",
            "planned",
            "high",
            "성과분석가 · 포트폴리오 관리자",
            [
                ("KRW·USD·코인·선물 포지션을 통화와 상품별로 분리해 위험 입력을 구성한다.", False),
                ("시점 정합 환율이 없으면 혼합 통화를 합산하지 않고 별도 결과로 표시한다.", False),
                ("저장 스냅샷 이력과 종목별 기여·스트레스 결과를 성과 화면에서 재생한다.", False),
            ],
            "amber",
        ),
        feature(
            "feat-paper-ledger-exit-reason-classification",
            "req-execution",
            "모의원장 청산 사유 구조화",
            "내부 모의계좌의 매도·청산 사건을 손절·익절·전략 신호·사용자 수동·기간 종료로 구조화한다.",
            "planned",
            "high",
            "주문원장 담당 · 리스크관리 총괄",
            [
                ("주문 요청과 체결 원장에 청산 사유 enum과 원인 사건 ID를 저장한다.", False),
                ("기존 매도 사건은 임의 추정하지 않고 manual 또는 unknown으로 호환 재생한다.", False),
                ("반복 손절 보호는 실제 stop_loss 사유만 집계하고 수동 매도를 손절로 오인하지 않는다.", False),
            ],
            "amber",
        ),
        feature(
            "feat-operations-live-database-restore",
            "feat-operations-backup",
            "사용자 승인형 운영 DB 복원",
            "검증 백업의 격리 사전검사 뒤에만 운영 SQLite 교체를 허용하고 실패 시 원본으로 롤백한다.",
            "planned",
            "critical",
            "감사로그·재현 조사 담당 · SRE",
            [
                ("복원 전 현재 DB와 대상 백업을 별도 보존하고 스키마·무결성·원장 재생을 재검사한다.", False),
                ("사용자 명시 승인과 앱 중지 상태에서만 원자적으로 교체한다.", False),
                ("실패 시 기존 DB를 복구하고 복원·롤백 증거를 민감정보 없이 남긴다.", False),
            ],
            "rose",
        ),
        feature(
            "feat-futures-official-product-lifecycle",
            "feat-execution-domestic-futures-paper-sandbox",
            "선물 공식 상품·일일정산·만기 생명주기",
            "사용자 입력 가정만 쓰는 내부 선물 sandbox에 공식 상품 마스터, 거래일, 일일정산, 만기와 롤오버를 별도 어댑터로 연결한다.",
            "planned",
            "high",
            "파생·펀딩 담당 · 시장데이터 엔지니어",
            [
                ("공식 상품코드·계약승수·호가단위·만기와 거래시간을 시점별 버전으로 저장한다.", False),
                ("일일정산과 증거금 변동을 append-only 사건으로 재생한다.", False),
                ("만기 청산과 롤오버를 새 주문으로 구분하고 자동 연장을 기본 금지한다.", False),
                ("외부 증권사 계좌가 없으면 내부 sandbox로만 표시하고 실제 주문을 보내지 않는다.", False),
            ],
            "amber",
        ),
        feature(
            "feat-codex-long-session-recovery-ux",
            "feat-trading-room-openai",
            "Codex 장시간 회의·실패 복구 UX",
            "긴 부서 회의에서 사용량·취소·부분 실패·앱 재시작 상태를 사용자에게 설명하고 입력과 완료 보고를 보존한다.",
            "in_progress",
            "high",
            "AI 오케스트레이션 담당",
            [
                ("취소·오류 뒤에도 사용자의 요청과 기존 대화를 보존하고 재시도할 수 있다.", True),
                ("장시간 실제 App Server 회의에서 한도·중단·부서별 부분 실패를 반복 검수한다.", False),
                ("재시작 후 손실된 Codex 실행을 이어 붙이지 않고 체크포인트에서 안전하게 재실행·종료한다.", False),
            ],
            "amber",
        ),
        feature(
            "feat-market-nasdaq-official-feed",
            "feat-trading-room-live-index-boards",
            "NASDAQ 공식 지수 공급자 연결",
            "라이선스와 재배포 범위를 확인한 공식 공급자로 NASDAQ 지수 값·등락·관측 시각을 전광판에 연결한다.",
            "planned",
            "medium",
            "시장데이터 엔지니어",
            [
                ("공식 공급자·라이선스·지연 여부와 호출 비용을 결정한다.", False),
                ("관측 시각·지연·오류·rate limit을 표시하고 숫자를 임의 보간하지 않는다.", False),
                ("공급자 미연결·만료 시 FEED WAIT로 돌아가며 주문 안전 상태와 분리한다.", False),
            ],
            "amber",
        ),
        feature(
            "feat-shadow-long-run-soak",
            "feat-stages-shadow",
            "장시간 섀도우 운용·재시작 내구 검사",
            "자동 섀도우 감시를 장시간 실행해 중복 후보, 메모리·타이머 누수, 공급자 만료와 재시작 대사를 검증한다.",
            "planned",
            "high",
            "매매운영 담당 · SRE",
            [
                ("최소 24시간 격리 운용에서 동일 완료봉 후보 중복이 발생하지 않는다.", False),
                ("공급자 만료·네트워크 오류·앱 재시작에서 신규 진입을 fail-closed로 잠근다.", False),
                ("메모리·타이머·SQLite 증가량과 복구 시간을 기록해 기준 초과를 경고한다.", False),
            ],
            "amber",
        ),
        feature(
            "feat-data-official-news-community-adapters",
            "feat-data-ingestion",
            "공식 뉴스·커뮤니티 공급자 어댑터",
            "현재의 정규화 계약과 Telegram 자료 외에 이용약관이 허용된 뉴스·커뮤니티 공급자를 읽기 전용으로 연결한다.",
            "planned",
            "medium",
            "뉴스·심리 분석가 · 시장데이터 엔지니어",
            [
                ("공식 API·사용자 내보내기·라이선스가 확인된 공급자만 채택한다.", False),
                ("뉴스 사실과 커뮤니티 심리·확산량을 분리하고 중복·봇 의심을 표시한다.", False),
                ("수집 실패·rate limit·유료 한도에서 기존 근거를 최신 정보처럼 재사용하지 않는다.", False),
            ],
            "amber",
        ),
        feature(
            "feat-pr-tistory-manual-publish-package",
            "req-public-relations",
            "티스토리 수동 게시 패키지 내보내기",
            "종료된 공식 글쓰기 API를 우회하지 않고 검수된 원고·이미지·캡션·대체텍스트를 사용자가 직접 게시할 패키지로 내보낸다.",
            "planned",
            "medium",
            "범니버스 개발기 작가 · 미디어 편집 담당",
            [
                ("대표 승인 리비전의 원고와 공개 허용 근거만 포함한다.", False),
                ("비밀정보·계좌·절대 로컬 경로·미허용 이미지와 시장데이터를 제외한다.", False),
                ("외부 자동 게시를 실행하지 않고 사용자 수동 복사·업로드 절차를 제공한다.", False),
            ],
            "amber",
        ),
        feature(
            "feat-projectstudio-status-reconciliation",
            "req-project-records",
            "ProjectStudio 구현 상태·노드 정합성 감사",
            "Investa 코드·테스트·문서와 기능명세·유저플로를 대조해 누락, 과장 완료, 미체크 구현과 중복 구조를 보수적으로 바로잡는다.",
            "done",
            "high",
            "프로젝트 관리자 · 테스트 엔지니어",
            [
                ("완료 노드는 모든 수용 기준이 체크되고 미완료 노드는 완료로 표시하지 않는다.", True),
                ("부모·연결 기능·유저플로 edge 무결성과 제목·정렬 중복을 검사한다.", True),
                ("기획·개발 문서에 명시된 잔여 작업 중 기능 노드가 없던 항목을 추가한다.", True),
                ("외부 계정·유료 공급자·모델 설치가 필요한 항목은 계획 상태로 유지한다.", True),
            ],
            "green",
        ),
    ]


DONE_ALL = {
    "req-research",
    "req-screening",
    "req-decision",
    "req-agents",
    "req-trading-room",
    "req-crypto",
    "req-risk",
    "req-execution",
    "req-performance",
    "req-operations",
    "feat-workspace-secrets",
    "feat-agents-specialists",
    "feat-trading-room-roster",
    "feat-trading-room-openai",
    "feat-dashboard-decision-detail",
    "feat-stages-shadow",
    "feat-crypto-leverage",
    "feat-risk-market-safety",
    "feat-records-reference",
    "feat-records-planning",
    "feat-records-design",
    "feat-records-development",
    "feat-records-design-skills",
    "feat-records-crypto-scope",
    "org-headquarters",
    "org-research",
    "org-strategy",
    "org-risk",
    "org-digital-assets",
    "org-investment-engineering",
    "role-investment-director",
    "role-research-director",
    "role-technical-analyst",
    "role-fundamental-analyst",
    "role-news-sentiment-analyst",
    "role-flow-macro-analyst",
    "role-paper-strategy-researcher",
    "role-strategy-director",
    "role-bull-researcher",
    "role-bear-researcher",
    "role-trader-planner",
    "role-strategy-researcher",
    "role-aggressive-risk",
    "role-neutral-risk",
    "role-conservative-risk",
    "role-digital-assets-director",
    "role-crypto-spot-analyst",
    "role-derivatives-funding-analyst",
    "role-onchain-microstructure-analyst",
    "role-pr-director",
    "role-development-writer",
    "role-fact-performance-editor",
    "role-media-editor",
    "role-evidence-archivist",
    "role-investment-engineering-director",
    "role-market-data-engineer",
    "role-quant-platform-engineer",
    "role-strategy-mlops-engineer",
    "role-trading-sre-security",
    "role-compliance-director",
    "role-algorithm-change-auditor",
    "role-trading-restriction-officer",
    "role-audit-replay-officer",
    "role-publication-data-compliance",
    "feat-workspace-startup",
    "feat-broker-market-adapter",
    "feat-broker-rate-limit",
    "feat-broker-connect",
    "feat-data-news-social-synthesis",
    "feat-data-telegram-feed",
    "feat-agents-trace",
    "feat-trading-room-drawer",
    "feat-trading-room-chat",
    "feat-crypto-spot",
    "feat-pr-evidence-pack",
    "feat-pr-draft",
    "feat-pr-media-review",
    "feat-pr-approval-publish",
    "role-alert-killswitch-operator",
    "feat-research-walk-forward",
    "feat-dashboard-overview",
}


PARTIAL_CHECKS: dict[str, set[int]] = {
    "req-organization": {0, 1, 3},
    "feat-workspace-settings": {0, 2},
    "feat-trading-room-hierarchy": {0, 1, 2, 3},
    "feat-account-bound-provider-verification": {0, 2},
    "feat-sec-live-contact-verification": {0},
    "feat-execution-reconcile": {2},
    "feat-stages-live-approval": {0, 2},
    "feat-crypto-risk-gate": {0, 1, 2},
    "feat-records-backend-implementation-roadmap": {1, 2, 3, 4, 5, 6, 7, 8, 9, 10},
    "feat-records-external-integration-priority": {0, 1, 2, 3, 4, 6, 7},
    "org-execution": {0},
    "org-public-relations": {1},
    "org-compliance-audit": {0, 2},
    "role-risk-director": {0, 1, 2},
    "role-risk-monitor": {0, 1, 2},
    "role-independent-model-validator": {0, 1, 2},
    "role-execution-director": {0, 2},
    "role-broker-adapter-operator": {0, 1, 2},
    "role-order-ledger-operator": {0, 2},
    "role-reconciliation-operator": {0, 2},
    "role-trade-quality-surveillance": {0, 1, 2, 3, 4},
    "role-crypto-operations-monitor": {0, 2},
    "feat-crypto-paper-terminal-provider": {0, 1, 2},
    "feat-forecast-foundation-model-adapters": {1},
}

NEW_DONE_ALL = {
    "feat-portfolio-risk-ledger-composition-ui",
    "feat-paper-ledger-exit-reason-classification",
    "feat-pr-tistory-manual-publish-package",
}

NEW_PARTIAL_CHECKS: dict[str, set[int]] = {
    "feat-strategy-protection-operations-visibility": {0, 1, 2},
    "feat-shadow-long-run-soak": {1, 2},
    "feat-codex-long-session-recovery-ux": {0, 2},
    "feat-futures-official-product-lifecycle": {1, 2, 3},
    "feat-market-nasdaq-official-feed": {1, 2},
    "feat-data-official-news-community-adapters": {1, 2},
}


FLOW_COMPLETED_IDS = {
    "flow-meeting-analysis-cycle-phase",
    "flow-meeting-analysis-cycle-6",
    "flow-meeting-analysis-cycle-7",
    "flow-remote-2",
    "flow-remote-3",
    "flow-remote-4",
    "flow-research-1",
}


def set_feature_checks(feature_item: dict[str, Any], met_indices: set[int]) -> None:
    for index, item in enumerate(feature_item["acceptanceCriteria"]):
        item["isMet"] = index in met_indices
    met_count = sum(bool(item["isMet"]) for item in feature_item["acceptanceCriteria"])
    total = len(feature_item["acceptanceCriteria"])
    if total > 0 and met_count == total:
        feature_item["status"] = "done"
    elif met_count > 0:
        feature_item["status"] = "in_progress"
    elif feature_item["status"] == "done":
        feature_item["status"] = "planned"


def normalize_feature_order(features: list[dict[str, Any]]) -> None:
    original_order = {item["id"]: index for index, item in enumerate(features)}
    children: dict[str | None, list[dict[str, Any]]] = {}
    for item in features:
        children.setdefault(item["parentId"], []).append(item)
    for values in children.values():
        values.sort(key=lambda item: original_order[item["id"]])
    ordered: list[dict[str, Any]] = []

    def visit(item: dict[str, Any]) -> None:
        ordered.append(item)
        for child in children.get(item["id"], []):
            visit(child)

    for root in children.get(None, []):
        visit(root)
    if len(ordered) != len(features):
        raise RuntimeError("기능 트리를 전부 순회하지 못했습니다.")
    for index, item in enumerate(ordered, start=1):
        item["sortOrder"] = index * 10
    features[:] = ordered


def normalize_prd(markdown: str, counts: dict[str, int]) -> str:
    lines = markdown.splitlines()
    section_start = next(
        (index for index, line in enumerate(lines) if line.startswith("## ") and AUDIT_SECTION_TITLE in line),
        None,
    )
    if section_start is not None:
        section_end = next(
            (index for index in range(section_start + 1, len(lines)) if lines[index].startswith("## ")),
            len(lines),
        )
        del lines[section_start:section_end]
    while lines and not lines[-1].strip():
        lines.pop()
    lines.extend(
        [
            "",
            f"## {AUDIT_SECTION_TITLE} (2026-08-25)",
            "- 완료는 코드 경로와 테스트 근거가 있고 모든 수용 기준이 충족된 기능만 사용한다. 일부만 구현된 기능은 진행 중, 계정·공급자·모델이 필요한 기능은 계획으로 유지한다.",
            "- 실제 구현된 44인 로스터와 역할별 Codex 정책·RoleReport 계약을 조직도 역할 노드에 반영했다. 외부 계좌 왕복·24시간 운영·전용 엔진이 필요한 직원 기능은 부분 체크로 남겼다.",
            "- 잘못 배치된 토스 계좌 잔고 UI를 브로커 대분류로 옮기고 기능 정렬 순서를 트리 기준으로 고유하게 다시 부여했다.",
            "- 중복된 PRD 장 번호를 현재 문서 순서대로 다시 매기고 기능 부모·유저플로 연결·edge 무결성을 재검사했다.",
            "- 누락됐던 보호 판정 운영 화면, 원장 기반 포트폴리오 위험 UI, 청산 사유, 운영 DB 복원, 선물 공식 생명주기, Codex 장시간 복구, NASDAQ 공급자, 섀도우 내구 검사, 공식 뉴스·커뮤니티 어댑터와 티스토리 수동 게시 패키지 노드를 추가했다.",
            f"- 감사 후 기능 노드 상태: 완료 {counts['done']}개, 진행 중 {counts['in_progress']}개, 계획 {counts['planned']}개.",
        ]
    )
    numbered: list[str] = []
    section_number = 0
    for line in lines:
        if line.startswith("## ") and not line.startswith("### "):
            section_number += 1
            title = re.sub(r"^\d+\.\s*", "", line[3:])
            line = f"## {section_number}. {title}"
        numbered.append(line)
    return "\n".join(numbered).rstrip()


def normalize_flow(current: dict[str, Any]) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    nodes: list[dict[str, Any]] = []
    for raw in current["userFlow"]["nodes"]:
        metadata = json.loads(raw.get("metadata_json") or "{}")
        if raw["id"] in FLOW_COMPLETED_IDS:
            metadata["isCompleted"] = True
        nodes.append(
            {
                "id": raw["id"],
                "laneId": raw["lane_id"],
                "title": raw["title"],
                "description": raw["description"],
                "kind": raw["kind"],
                "positionX": raw["position_x"],
                "positionY": raw["position_y"],
                "colorKey": raw.get("color_key") or "violet",
                "depth": raw.get("depth"),
                "parentId": raw.get("parent_id"),
                "linkedFeatureIds": json.loads(raw.get("linked_feature_ids") or "[]"),
                "branchCondition": raw.get("branch_condition"),
                "inputArtifacts": metadata.get("inputArtifacts", []),
                "outputArtifacts": metadata.get("outputArtifacts", []),
                "methods": metadata.get("methods", []),
                "validation": metadata.get("validation", ""),
                "failureHandling": metadata.get("failureHandling", ""),
                "codePaths": metadata.get("codePaths", []),
                "testPaths": metadata.get("testPaths", []),
                "completionCriteria": metadata.get("completionCriteria", ""),
                "isCompleted": bool(metadata.get("isCompleted", False)),
            }
        )
    edges = [
        {
            "id": edge["id"],
            "sourceNodeId": edge["source_node_id"],
            "targetNodeId": edge["target_node_id"],
        }
        for edge in current["userFlow"]["edges"]
    ]
    return nodes, edges


def validate_state(features: list[dict[str, Any]], nodes: list[dict[str, Any]], edges: list[dict[str, Any]]) -> None:
    feature_ids = {item["id"] for item in features}
    if len(feature_ids) != len(features):
        raise RuntimeError("중복 기능 ID가 있습니다.")
    for item in features:
        if item["parentId"] is not None and item["parentId"] not in feature_ids:
            raise RuntimeError(f"기능 부모가 없습니다: {item['id']} -> {item['parentId']}")
        checks = item["acceptanceCriteria"]
        all_met = bool(checks) and all(check["isMet"] for check in checks)
        if item["status"] == "done" and not all_met:
            raise RuntimeError(f"완료 기능에 미충족 기준이 있습니다: {item['id']}")
        if item["status"] != "done" and all_met:
            raise RuntimeError(f"모든 기준이 충족됐지만 완료가 아닙니다: {item['id']}")
    node_ids = {node["id"] for node in nodes}
    for node in nodes:
        missing = set(node["linkedFeatureIds"]) - feature_ids
        if missing:
            raise RuntimeError(f"유저플로 연결 기능이 없습니다: {node['id']} -> {sorted(missing)}")
    for edge in edges:
        if edge["sourceNodeId"] not in node_ids or edge["targetNodeId"] not in node_ids:
            raise RuntimeError(f"유저플로 edge 대상이 없습니다: {edge['id']}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("projectstudio_root", type=Path)
    parser.add_argument("--commit", action="store_true")
    args = parser.parse_args()
    api = load_api(args.projectstudio_root)
    database = api.default_database_path()
    with api.connect_database(database, writable=args.commit) as connection:
        current = api.get_project(connection, PROJECT_ID)
        features = copy.deepcopy(current["features"])
        by_id = {item["id"]: item for item in features}

        by_id["investa-root"]["description"] = (
            "한국·미국 주식·암호화폐·내부 선물 sandbox의 분석과 결정론적 위험 통제를 "
            "내부 모의원장에 연결하는 로컬 데스크톱 프로그램. 외부 모의계좌와 실전 주문은 별도 검증 전까지 잠근다."
        )
        root_checks = by_id["investa-root"]["acceptanceCriteria"]
        for item in root_checks:
            item["isMet"] = item["id"] != "investa-root-ac-external"
        if not any(item["id"] == "investa-root-ac-external" for item in root_checks):
            root_checks.append(
                {
                    "id": "investa-root-ac-external",
                    "description": "외부 모의계좌 왕복 검증과 장시간 운용을 마친 뒤에도 실전 주문은 별도 승인 없이는 열리지 않는다.",
                    "isMet": False,
                    "sortOrder": len(root_checks),
                }
            )
        by_id["investa-root"]["status"] = "in_progress"

        by_id["feat-broker-toss-account-balance-ui"]["parentId"] = "req-broker"
        by_id["feat-execution-ledger"]["description"] = (
            "내부 SQLite 모의주문의 명령·승인·제출·체결 사건과 포지션을 append-only 원장에 기록한다. "
            "외부 브로커 부분체결·환율 대사는 별도 외부 연결 기능으로 분리한다."
        )
        crypto_scope_checks = by_id["feat-records-crypto-scope"]["acceptanceCriteria"]
        crypto_scope_checks[1]["description"] = (
            "정식 조직의 디지털자산부 5명을 포함해 전체 44인 조직과 27인 MVP 목표를 현재 조직도 결정에 연결한다."
        )
        crypto_scope_checks[2]["description"] = (
            "Upbit·Binance 공급자와 현물·코인 선물 경계를 공식 API·키 권한·비용·sandbox 기준으로 분리 기록한다."
        )

        account_hub = by_id["feat-workspace-account-connection-hub"]
        account_hub["description"] = (
            "설정에서 국장·미장·코인·증권 선물·코인 선물을 분리하고 공급자별 지원 범위와 실제 연결 상태를 표시한다. "
            "긴 연결 페이지는 키보드로 조작 가능한 접기·펼치기 섹션으로 구성하며 닫아도 입력 상태를 유지한다."
        )
        if not any(item["id"] == "ac-account-hub-4" for item in account_hub["acceptanceCriteria"]):
            account_hub["acceptanceCriteria"].append(
                {
                    "id": "ac-account-hub-4",
                    "description": "시장·뉴스·증권사·거래소·AI 연결 페이지를 개별 접기·펼치기로 탐색하고 키보드 포커스로 조작한다.",
                    "isMet": True,
                    "sortOrder": len(account_hub["acceptanceCriteria"]),
                }
            )

        analysis_records = by_id["feat-analysis-generic-records"]
        analysis_records["description"] = (
            "성공 백테스트뿐 아니라 실행 차단 연구와 부서장 회의 종합을 상태·시장·요청·완료 시각과 함께 SQLite 불변 기록으로 저장한다. "
            "국장·미장·코인·증권 선물·코인 선물 분류와 예측 자산 계약 필터를 동일한 분석 보관소에서 조회한다."
        )
        if not any(item["id"] == "ac-analysis-generic-3" for item in analysis_records["acceptanceCriteria"]):
            analysis_records["acceptanceCriteria"].append(
                {
                    "id": "ac-analysis-generic-3",
                    "description": "증권 선물과 코인 선물 분석을 별도 시장으로 저장·필터링하고 기존 기록을 보존하는 스키마 마이그레이션을 검증한다.",
                    "isMet": True,
                    "sortOrder": len(analysis_records["acceptanceCriteria"]),
                }
            )

        for feature_id in DONE_ALL:
            if feature_id not in by_id:
                raise RuntimeError(f"완료 갱신 대상 기능이 없습니다: {feature_id}")
            set_feature_checks(by_id[feature_id], set(range(len(by_id[feature_id]["acceptanceCriteria"]))))
        for feature_id, met_indices in PARTIAL_CHECKS.items():
            if feature_id not in by_id:
                raise RuntimeError(f"부분 갱신 대상 기능이 없습니다: {feature_id}")
            set_feature_checks(by_id[feature_id], met_indices)

        for item in new_features():
            if item["id"] in by_id:
                existing_index = next(index for index, value in enumerate(features) if value["id"] == item["id"])
                features[existing_index] = item
            else:
                features.append(item)
            by_id[item["id"]] = item

        for feature_id in NEW_DONE_ALL:
            set_feature_checks(by_id[feature_id], set(range(len(by_id[feature_id]["acceptanceCriteria"]))))
        for feature_id, met_indices in NEW_PARTIAL_CHECKS.items():
            set_feature_checks(by_id[feature_id], met_indices)

        normalize_feature_order(features)
        counts = {status: sum(item["status"] == status for item in features) for status in ("done", "in_progress", "planned")}
        markdown = normalize_prd(current["project"]["prd_markdown"], counts)
        nodes, edges = normalize_flow(current)
        validate_state(features, nodes, edges)

        current_nodes, current_edges = normalize_flow(current)
        if (
            current["features"] == features
            and current["project"]["prd_markdown"] == markdown
            and current_nodes == nodes
            and current_edges == edges
        ):
            print(
                json.dumps(
                    {
                        "projectId": PROJECT_ID,
                        "committed": False,
                        "message": "동일한 구현 상태 감사 결과가 이미 반영되어 있습니다.",
                        "statusCounts": counts,
                    },
                    ensure_ascii=False,
                    indent=2,
                )
            )
            return

        bundle = {
            "schemaVersion": 1,
            "projectId": PROJECT_ID,
            "expectedPrdRevisionNumber": current["project"]["revision_number"],
            "prd": {"title": current["project"]["prd_title"], "markdown": markdown},
            "features": features,
            "userFlow": {"nodes": nodes, "edges": edges},
        }
        result = api.apply_bundle(connection, database, bundle, commit=args.commit)
        result["statusCounts"] = counts
        result["addedOrReconciledFeatureCount"] = len(new_features())
        print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
