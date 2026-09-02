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
    "feat-crypto-binance-private-verification",
    "feat-workspace-settings",
}


PARTIAL_CHECKS: dict[str, set[int]] = {
    "req-organization": {0, 1, 3},
    "feat-trading-room-hierarchy": {0, 1, 2, 3},
    "feat-account-bound-provider-verification": {0, 2},
    "feat-sec-live-contact-verification": {0},
    "feat-execution-reconcile": {2},
    "feat-stages-live-approval": {0, 2},
    "feat-crypto-risk-gate": {0, 1, 2},
    "feat-records-backend-implementation-roadmap": {1, 2, 3, 4, 5, 6, 7, 8, 9, 10},
    "feat-records-external-integration-priority": {0, 1, 2, 3, 4, 5, 6, 7},
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
    "feat-data-ingestion": {0, 1},
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
}


FLOW_COMPLETION_OVERRIDES = {
    "flow-meeting-analysis-cycle-phase": False,
    "flow-meeting-analysis-cycle-1": True,
    "flow-meeting-analysis-cycle-symbol": True,
    "flow-meeting-analysis-cycle-preflight": True,
    "flow-meeting-analysis-cycle-evidence": True,
    "flow-meeting-analysis-cycle-2": True,
    "flow-meeting-analysis-cycle-3": True,
    "flow-meeting-analysis-cycle-4": True,
    "flow-meeting-analysis-cycle-5": True,
    "flow-meeting-analysis-cycle-6": True,
    "flow-meeting-analysis-cycle-save": True,
    "flow-meeting-analysis-cycle-risk": True,
    # 고정 fixture로 분석→백테스트→후보→사용자 승인→append-only 원장까지 자동
    # 골든패스가 통과한다. 실제 공급자 신호와 장시간 섀도우 증거가 필요한 전체
    # 레인·후보 생성·감시 단계는 과장하지 않고 미완료로 유지한다.
    "flow-meeting-analysis-cycle-7": False,
    "flow-meeting-analysis-cycle-approval": True,
    "flow-meeting-analysis-cycle-execution": True,
    "flow-meeting-analysis-cycle-ledger": True,
    "flow-meeting-analysis-cycle-shadow": False,
    "flow-meeting-analysis-cycle-symbol-error": True,
    "flow-meeting-analysis-cycle-provider-error": True,
    "flow-meeting-analysis-cycle-partial-failure": True,
    "flow-meeting-analysis-cycle-codex-recovery": False,
    "flow-meeting-analysis-cycle-risk-rejected": True,
    "flow-meeting-analysis-cycle-duplicate": True,
    "flow-remote-2": True,
    "flow-remote-3": True,
    "flow-remote-4": True,
    "flow-research-1": True,
}


def meeting_flow_nodes() -> list[dict[str, Any]]:
    lane_id = "lane-meeting-analysis-cycle"

    def node(
        node_id: str,
        title: str,
        description: str,
        kind: str,
        x: float,
        y: float,
        linked_feature_ids: list[str],
        *,
        branch_condition: str | None = None,
        code_paths: list[str] | None = None,
        test_paths: list[str] | None = None,
        completion_criteria: str = "",
    ) -> dict[str, Any]:
        return {
            "id": node_id,
            "laneId": lane_id,
            "title": title,
            "description": description,
            "kind": kind,
            "positionX": x,
            "positionY": y,
            "colorKey": "cyan",
            "depth": None,
            "parentId": None,
            "linkedFeatureIds": linked_feature_ids,
            "branchCondition": branch_condition,
            "inputArtifacts": [],
            "outputArtifacts": [],
            "methods": [],
            "validation": "",
            "failureHandling": "",
            "codePaths": code_paths or [],
            "testPaths": test_paths or [],
            "completionCriteria": completion_criteria,
            "isCompleted": FLOW_COMPLETION_OVERRIDES[node_id],
        }

    main_y = 4380.0
    branch_y = 4490.0
    return [
        node("flow-meeting-analysis-cycle-phase", "분석 요청부터 내부 모의원장까지", "사용자 분석 요청이 근거 수집·부서 회의·위험 심사·사용자 승인·내부 모의체결·원장 반영으로 이어지는 전체 사용 흐름이다.", "phase", 20, main_y, ["req-trading-room", "req-risk", "req-execution"], completion_criteria="분석 회의에서 생성된 후보가 사용자 승인 뒤 내부 원장과 성과 화면에 한 번만 반영되고 재시작 후 재생된다."),
        node("flow-meeting-analysis-cycle-1", "분석 안건 입력", "사용자가 종목·시장·보유 포지션·투자 판단 요청을 자연어로 입력한다.", "screen", 250, main_y, ["feat-trading-room-openai"], code_paths=["src/App.tsx"]),
        node("flow-meeting-analysis-cycle-symbol", "종목·보유 포지션 식별", "종목명·티커를 하나의 상장 법인과 시장으로 확정하고 연결 계좌의 익명화된 보유 수량·평단을 같은 기준 시각으로 찾는다.", "decision", 480, main_y, ["feat-broker-symbol-autocomplete", "feat-meeting-evidence-pack"], branch_condition="하나의 종목과 포지션을 확정하면 연결 사전점검으로 진행", code_paths=["src/meetingEvidence.ts", "src-tauri/src/market_data.rs"], test_paths=["scripts/meetingEvidence.test.ts", "scripts/analysisSnapshotRouting.test.ts"]),
        node("flow-meeting-analysis-cycle-preflight", "데이터 연결·신선도 사전점검", "시장가격·계좌·재무·공시·뉴스·Telegram 공급자의 설정, 기준 시각, 결측과 stale 상태를 분석 전에 확인한다.", "decision", 710, main_y, ["feat-official-news-community-adapters", "feat-analysis-point-in-time-price-snapshot"], branch_condition="필수 근거가 준비되면 수집, 아니면 결측을 명시하고 보류·부분 분석", code_paths=["src/connectionStatus.ts", "src-tauri/src/market_data.rs"], test_paths=["scripts/connectionStatus.test.ts", "scripts/meetingEvidence.test.ts"]),
        node("flow-meeting-analysis-cycle-evidence", "PIT 근거 묶음 수집", "현재가·차트·기술지표·익명화 포지션·재무·공시·뉴스·선택 Telegram을 한 번 수집하고 근거 ID와 관측 시각을 고정한다.", "action", 940, main_y, ["feat-meeting-evidence-pack", "feat-data-evidence-normalization"], code_paths=["src/meetingEvidence.ts", "src-tauri/src/market_data.rs"], test_paths=["scripts/meetingEvidence.test.ts"]),
        node("flow-meeting-analysis-cycle-2", "관련 부서 자동 선택·소집", "안건을 분류해 필요한 부서만 소집하고 위험·감사처럼 필수 안전 부서를 결정론적으로 보강한다.", "action", 1170, main_y, ["feat-trading-room-department-result-aggregation"], code_paths=["src-tauri/src/codex.rs", "src/App.tsx"]),
        node("flow-meeting-analysis-cycle-3", "직원별 역할 한정 분석", "선택 부서의 직원들이 같은 근거 묶음을 사용해 자기 역할 범위의 보고만 생성한다.", "action", 1400, main_y, ["feat-trading-room-role-scoped-task", "feat-codex-analysis-quality-profile"], code_paths=["src-tauri/src/codex.rs", "src/meetingEvidence.ts"], test_paths=["scripts/meetingEvidence.test.ts"]),
        node("flow-meeting-analysis-cycle-4", "완료 부서장부터 복귀", "실제 직원 완료 상태의 평균이 100%인 부서장부터 본부장실로 복귀하고 미완료 부서는 계속 작업한다.", "action", 1630, main_y, ["feat-trading-room-active-work-indicators"], code_paths=["src/App.tsx"]),
        node("flow-meeting-analysis-cycle-5", "부장 보고·이견 확인", "부장은 직원 보고만 종합하고 Bull·Bear·위험·데이터 결측과 근거 ID를 회의 패널에 표시한다.", "screen", 1860, main_y, ["feat-trading-room-department-result-aggregation"]),
        node("flow-meeting-analysis-cycle-6", "본부장 최종 종합", "본부장은 동일 원본 근거와 부서 보고를 재대조해 후보·보류·기각 및 확신도 근거를 구조화한다.", "decision", 2090, main_y, ["feat-codex-analysis-quality-profile"], code_paths=["src-tauri/src/codex.rs"]),
        node("flow-meeting-analysis-cycle-save", "분석 보관함 저장", "요청·완료 시각, 시장, 종목, 보고 상태, 근거와 차트 주석을 불변 분석 기록으로 저장한다.", "result", 2320, main_y, ["feat-analysis-generic-records", "feat-research-analysis-vault"], code_paths=["src-tauri/src/persistence.rs", "src/AnalysisWorkspace.tsx"]),
        node("flow-meeting-analysis-cycle-risk", "결정론적 위험 게이트", "LLM과 분리된 코드가 신선도·유동성·손실 한도·중복 주문·실전 잠금을 판정한다.", "decision", 2550, main_y, ["feat-risk-pretrade", "feat-risk-market-safety"], branch_condition="모든 안전 조건을 통과한 경우에만 내부 모의주문 후보 생성", code_paths=["src-tauri/src/execution_control.rs", "src-tauri/src/trading.rs"]),
        node("flow-meeting-analysis-cycle-7", "백테스트·신호 감시·후보 또는 보류", "회의 분석 ID와 단일 종목·지원 전략을 불변 계보로 저장하고 실제 공급자 완료 봉 백테스트를 실행한다. 같은 분석·종목의 실험만 60초 섀도우 감시에 연결하며 현재 신호가 있을 때만 사용자 승인 대기 내부 후보를 만들고, 신호 없음·지원 밖 시장·위험 실패는 사유와 함께 보류한다.", "result", 2780, main_y, ["feat-execution-internal-paper-candidate"], code_paths=["src/App.tsx", "src/meetingBacktest.ts", "src-tauri/src/meeting_handoff.rs", "src-tauri/src/operations.rs"], test_paths=["scripts/meetingBacktest.test.ts", "src-tauri/src/meeting_handoff.rs", "src-tauri/src/operations.rs"], completion_criteria="실제 회의 분석 ID가 백테스트 실험과 섀도우 후보의 원인 사건 ID로 저장되고 사용자 승인 뒤 내부 원장에 한 번 체결된 통합 실행 증거가 있다."),
        node("flow-meeting-analysis-cycle-approval", "사용자 승인", "사용자가 후보의 종목·방향·수량·가격·비용·위험 사유를 확인하고 승인 또는 거절한다.", "decision", 3010, main_y, ["feat-decision-portfolio-manager", "feat-remote-local-approval"], branch_condition="명시적 승인만 내부 모의체결로 진행"),
        node("flow-meeting-analysis-cycle-execution", "내부 모의체결", "SHADOW ONLY 경계에서 시장 세션과 비용 규칙을 적용해 SQLite 내부 모의계좌에만 체결한다.", "action", 3240, main_y, ["feat-execution-manual-paper-orders", "feat-execution-internal-paper-candidate"], code_paths=["src-tauri/src/paper_trading.rs"]),
        node("flow-meeting-analysis-cycle-ledger", "원장·예수금·성과 반영", "체결 사건을 append-only 원장에 한 번 기록하고 예수금·포지션·실현손익·분석 결과를 같은 사건 계보로 갱신한다.", "result", 3470, main_y, ["feat-execution-ledger", "feat-execution-ledger-ui"], code_paths=["src-tauri/src/persistence.rs", "src-tauri/src/paper_trading.rs"]),
        node("flow-meeting-analysis-cycle-shadow", "섀도우 감시 또는 종료", "승인된 전략은 최신 완료 봉을 감시하고, 단건 분석은 저장된 결과와 원장 상태를 보여준 뒤 종료한다.", "result", 3700, main_y, ["feat-stages-shadow", "feat-stages-shadow-live-bars"], completion_criteria="분석 원인 사건부터 감시·후보·원장까지 재시작 후에도 중복 없이 재생된다."),
        node("flow-meeting-analysis-cycle-symbol-error", "종목 불명확·보유 없음", "동명 법인·시장 불명확 또는 연결 계좌에 포지션이 없으면 후보를 만들지 않고 사용자가 종목·계좌를 다시 선택한다.", "result", 480, branch_y, ["feat-broker-symbol-autocomplete"], branch_condition="종목 또는 포지션을 확정하지 못함"),
        node("flow-meeting-analysis-cycle-provider-error", "공급자 결측·stale", "필수 공급자가 미설정·오류·stale이면 결측 근거를 표시하고 보류하거나 사용자가 허용한 부분 분석만 수행한다.", "result", 710, branch_y, ["feat-official-news-community-adapters", "feat-analysis-point-in-time-price-snapshot"], branch_condition="필수 근거의 신선도 또는 연결 조건 미충족"),
        node("flow-meeting-analysis-cycle-partial-failure", "부서 부분 실패·재시작", "완료 보고는 체크포인트로 보존하고 실패·미완료 부서만 재실행하거나 사용자가 회의를 취소한다.", "action", 1400, branch_y, ["feat-operations-meeting-checkpoint-recovery"], branch_condition="일부 직원·부서 작업 실패 또는 앱 재시작", code_paths=["src/workflowRecovery.ts", "src-tauri/src/persistence.rs"], test_paths=["scripts/workflowRecovery.test.ts"]),
        node("flow-meeting-analysis-cycle-codex-recovery", "Codex 한도·취소 복구", "입력과 검증된 완료 보고를 보존한 채 취소·한도·응답 계약 오류를 설명하고 안전하게 재시도한다.", "action", 2090, branch_y, ["feat-codex-long-session-recovery-ux"], branch_condition="Codex 실행 취소·한도·계약 검증 실패"),
        node("flow-meeting-analysis-cycle-risk-rejected", "위험 기각·보류", "위험 조건이 하나라도 실패하면 주문 후보를 만들지 않고 기각 사유를 분석 기록에 남긴다.", "result", 2550, branch_y, ["feat-risk-pretrade", "feat-analysis-generic-records"], branch_condition="결정론적 위험 게이트 실패"),
        node("flow-meeting-analysis-cycle-duplicate", "중복 주문 차단·원장 대사", "같은 원인 사건의 후보·체결 재전송을 차단하고 불일치가 있으면 체결을 중단한 채 원장을 재생한다.", "result", 3240, branch_y, ["feat-execution-ledger", "feat-execution-internal-paper-candidate"], branch_condition="idempotency 충돌 또는 원장 대사 실패"),
    ]


def meeting_flow_edges() -> list[dict[str, str]]:
    pairs = [
        ("phase", "1"), ("1", "symbol"), ("symbol", "preflight"),
        ("preflight", "evidence"), ("evidence", "2"), ("2", "3"),
        ("3", "4"), ("4", "5"), ("5", "6"), ("6", "save"),
        ("save", "risk"), ("risk", "7"), ("7", "approval"),
        ("approval", "execution"), ("execution", "ledger"), ("ledger", "shadow"),
        ("symbol", "symbol-error"), ("preflight", "provider-error"),
        ("3", "partial-failure"), ("partial-failure", "3"),
        ("6", "codex-recovery"), ("codex-recovery", "6"),
        ("risk", "risk-rejected"), ("execution", "duplicate"),
    ]
    prefix = "flow-meeting-analysis-cycle-"
    return [
        {"id": f"edge-meeting-analysis-cycle-{index + 1}", "sourceNodeId": f"{prefix}{source}", "targetNodeId": f"{prefix}{target}"}
        for index, (source, target) in enumerate(pairs)
    ]


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
            f"## {AUDIT_SECTION_TITLE} (2026-08-31)",
            "- 완료는 코드 경로와 테스트 근거가 있고 모든 수용 기준이 충족된 기능만 사용한다. 일부만 구현된 기능은 진행 중, 계정·공급자·모델이 필요한 기능은 계획으로 유지한다.",
            "- 실제 구현된 44인 로스터와 역할별 Codex 정책·RoleReport 계약을 조직도 역할 노드에 반영했다. 외부 계좌 왕복·24시간 운영·전용 엔진이 필요한 직원 기능은 부분 체크로 남겼다.",
            "- 잘못 배치된 토스 계좌 잔고 UI를 브로커 대분류로 옮기고 기능 정렬 순서를 트리 기준으로 고유하게 다시 부여했다.",
            "- 중복된 PRD 장 번호를 현재 문서 순서대로 다시 매기고 기능 부모·유저플로 연결·edge 무결성을 재검사했다.",
            "- 누락됐던 보호 판정 운영 화면, 원장 기반 포트폴리오 위험 UI, 청산 사유, 운영 DB 복원, 선물 공식 생명주기, Codex 장시간 복구, NASDAQ 공급자, 섀도우 내구 검사와 티스토리 수동 게시 패키지 노드를 추가했다.",
            "- 중복이던 뉴스·커뮤니티 어댑터는 공급자 선정 하위의 정식 노드 하나로 합치고 데이터 수집 하위 레거시 노드는 제거했다.",
            "- 소셜 로그인 전송 구현과 계정 생명주기 정책을 분리하고, WebSocket·Telegram·KIS 차트 어댑터의 구현 완료와 실제 장시간·자격정보 왕복을 서로 다른 기준으로 나눴다.",
            "- 분석 요청부터 근거 수집·부서 회의·분석 보관·위험 판정·사용자 승인·내부 체결·원장·섀도우까지 한 유저플로우로 연결하고 실패·복구 분기를 추가했다. 실제 엔진 통합 실행이 없는 후보 이후 단계와 레인 전체는 미완료로 교정했다.",
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


def normalize_flow(
    current: dict[str, Any], *, reconcile: bool
) -> tuple[list[dict[str, Any]], list[dict[str, str]]]:
    nodes: list[dict[str, Any]] = []
    for raw in current["userFlow"]["nodes"]:
        metadata = json.loads(raw.get("metadata_json") or "{}")
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
    if not reconcile:
        return nodes, edges

    by_id = {node["id"]: node for node in nodes}
    for node_id, is_completed in FLOW_COMPLETION_OVERRIDES.items():
        if node_id in by_id:
            by_id[node_id]["isCompleted"] = is_completed

    meeting_nodes = meeting_flow_nodes()
    meeting_ids = {node["id"] for node in meeting_nodes}
    nodes = [node for node in nodes if node["id"] not in meeting_ids]
    nodes.extend(meeting_nodes)
    edges = [
        edge
        for edge in edges
        if edge["sourceNodeId"] not in meeting_ids
        and edge["targetNodeId"] not in meeting_ids
    ]
    edges.extend(meeting_flow_edges())
    # ProjectStudio 조회 순서와 맞춰 두 번째 실행이 리비전을 만들지 않게 한다.
    nodes.sort(key=lambda node: (node["laneId"], node["positionX"], node["id"]))
    edges.sort(key=lambda edge: edge["id"])
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
        legacy_news_feature_id = "feat-data-official-news-community-adapters"
        canonical_news_feature_id = "feat-official-news-community-adapters"
        if any(item["id"] == legacy_news_feature_id for item in features):
            if not any(item["id"] == canonical_news_feature_id for item in features):
                raise RuntimeError("정식 뉴스·커뮤니티 어댑터 기능 없이 레거시 중복 노드만 존재합니다.")
            features = [item for item in features if item["id"] != legacy_news_feature_id]
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

        # OAuth 전송 구현은 보안 기능 노드 하나에서만 추적한다. 이전 작업공간
        # 노드는 아직 남은 계정 생명주기 정책으로 좁혀 중복 개발을 막는다.
        social_lifecycle = by_id["feat-workspace-social-auth-expansion"]
        social_lifecycle["title"] = "연결 계정 생명주기·복구 정책"
        social_lifecycle["description"] = (
            "Google OAuth 구현과 별개로 연결 계정 해제, 소유자 복구, 세션 만료, 탈퇴와 로컬 데이터 "
            "보존·삭제 정책을 확정한다. 인증 전송 구현은 feat-security-social-login에서만 추적한다."
        )
        social_lifecycle["acceptanceCriteria"] = [
            {
                "id": "ac-social-auth-lifecycle-1",
                "description": "연결 계정 해제·세션 만료·소유자 복구 시나리오와 재인증 조건을 확정한다.",
                "isMet": True,
                "sortOrder": 0,
            },
            {
                "id": "ac-social-auth-lifecycle-2",
                "description": "탈퇴 시 로컬 데이터·보안 저장소 식별자·백업의 보존과 삭제 범위를 확정하고 검증한다.",
                "isMet": True,
                "sortOrder": 1,
            },
        ]
        social_lifecycle["status"] = "done"

        # 구현과 실제 공급자 장시간 왕복을 하나의 미체크 기준으로 섞지 않는다.
        realtime = by_id["feat-official-realtime-stream-adapters"]
        realtime["acceptanceCriteria"][5]["isMet"] = True

        aggregation = by_id["feat-auto-realtime-aggregation"]
        aggregation["acceptanceCriteria"][6].update(
            {
                "description": "토스 공식 101 handshake와 국장 시장 topic 구독 ack를 실제 연결에서 확인한다.",
                "isMet": True,
            }
        )
        if not any(
            item["id"] == "feat-auto-realtime-aggregation-ac-8"
            for item in aggregation["acceptanceCriteria"]
        ):
            aggregation["acceptanceCriteria"].append(
                {
                    "id": "feat-auto-realtime-aggregation-ac-8",
                    "description": "국장·미장 장중 체결·호가와 자산별 24시간 공식 공급자 왕복을 검증한다.",
                    "isMet": False,
                    "sortOrder": len(aggregation["acceptanceCriteria"]),
                }
            )

        news = by_id["feat-official-news-community-adapters"]
        news["acceptanceCriteria"][3]["isMet"] = True
        news["acceptanceCriteria"][2].update(
            {
                "description": "사용자 MTProto 자격정보로 선택 Telegram 방송 채널의 실제 읽기 전용 수집을 검증한다.",
                "isMet": True,
            }
        )
        if not any(
            item["id"] == "feat-official-news-community-adapters-ac-6"
            for item in news["acceptanceCriteria"]
        ):
            news["acceptanceCriteria"].append(
                {
                    "id": "feat-official-news-community-adapters-ac-6",
                    "description": "사용자 SEC 연락처로 대표 미국 종목의 재무·공시 실제 왕복을 반복 검증한다.",
                    "isMet": False,
                    "sortOrder": len(news["acceptanceCriteria"]),
                }
            )

        ai_providers = by_id["feat-ai-provider-analysis-adapters"]
        ai_providers["acceptanceCriteria"][5].update(
            {
                "description": "Claude·Antigravity 응답을 44인 공통 RoleReport·DepartmentReport 서버 계약으로 검증한다.",
                "isMet": True,
            }
        )
        if not any(
            item["id"] == "feat-ai-provider-analysis-adapters-ac-7"
            for item in ai_providers["acceptanceCriteria"]
        ):
            ai_providers["acceptanceCriteria"].append(
                {
                    "id": "feat-ai-provider-analysis-adapters-ac-7",
                    "description": "외부 AI의 직원별 작업 상태 이벤트·취소·부서 집계 실행을 공통 오케스트레이션에 연결한다.",
                    "isMet": True,
                    "sortOrder": len(ai_providers["acceptanceCriteria"]),
                }
            )
        else:
            next(
                item
                for item in ai_providers["acceptanceCriteria"]
                if item["id"] == "feat-ai-provider-analysis-adapters-ac-7"
            ).update(
                {
                    "description": "외부 AI의 직원별 작업 상태 이벤트·취소·부서 집계 실행을 공통 오케스트레이션에 연결한다.",
                    "isMet": True,
                }
            )

        cross_asset = by_id["feat-cross-asset-chart-annotation-contracts"]
        cross_asset["acceptanceCriteria"][4].update(
            {
                "description": "KIS 공식 국내선물 계약별 일봉 어댑터가 계약코드·세션·PIT 시각을 보존한다.",
                "isMet": True,
            }
        )
        if not any(
            item["id"] == "ac-cross-chart-securities-provider-roundtrip"
            for item in cross_asset["acceptanceCriteria"]
        ):
            cross_asset["acceptanceCriteria"].append(
                {
                    "id": "ac-cross-chart-securities-provider-roundtrip",
                    "description": "KIS 자격정보가 있는 환경에서 국내선물 공식 응답과 차트 근거의 실제 왕복을 검증한다.",
                    "isMet": False,
                    "sortOrder": len(cross_asset["acceptanceCriteria"]),
                }
            )

        # 실제 Google 계정 거부→연결→재로그인 왕복과 토스 읽기 전용 어댑터는
        # 저장소 문서·회귀 테스트에 근거가 있으므로 중복 미완료로 남기지 않는다.
        by_id["feat-security-social-login"]["acceptanceCriteria"][4]["isMet"] = True
        by_id["feat-records-external-integration-priority"]["acceptanceCriteria"][5]["isMet"] = True

        workspace_settings = by_id["feat-workspace-settings"]
        workspace_settings["acceptanceCriteria"][1]["description"] = (
            "토스 공식 KR·US 캘린더의 휴장·부분 세션·미국 익일 종료를 조회하고 운영 화면에 표시한다."
        )
        if not any(
            item["id"] == "ac-workspace-settings-order-session-gate"
            for item in workspace_settings["acceptanceCriteria"]
        ):
            workspace_settings["acceptanceCriteria"].append(
                {
                    "id": "ac-workspace-settings-order-session-gate",
                    "description": "공식 정규장 세션을 국장·미장 시장가 즉시 내부 모의체결에 fail-closed 방식으로 반영하고 장외에는 지정가 대기 주문만 허용한다.",
                    "isMet": False,
                    "sortOrder": len(workspace_settings["acceptanceCriteria"]),
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
        nodes, edges = normalize_flow(current, reconcile=True)
        validate_state(features, nodes, edges)

        current_nodes, current_edges = normalize_flow(current, reconcile=False)
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
