"""전체 보유 포트폴리오 분석과 사용자 운용 원칙을 ProjectStudio에 멱등 반영한다."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path


PROJECT_ID = "36e87491-74a8-48ca-a7b8-30fa6ccea131"
FEATURE_IDS = {
    "feat-holdings-portfolio-analysis",
    "feat-user-portfolio-mandate",
    "feat-account-portfolio-presentation",
    "feat-telegram-evidence-autosync",
    "feat-complete-investment-analysis-report",
}
FLOW_NODE_ID = "flow-meeting-analysis-cycle-portfolio"
PRESENTATION_NODE_ID = "flow-account-portfolio-presentation"
TELEGRAM_NODE_ID = "flow-telegram-evidence-autosync"
SECTION_MARKER = "전체 보유 포트폴리오 분석과 사용자 운용 원칙"
PRESENTATION_MARKER = "실계좌 보유자산 표현과 Telegram 근거 자동 연결"
OUTPUT_CONTRACT_MARKER = "10영역 투자 분석 출력 계약"


def load_api(root: Path):
    source = root / "scripts" / "projectstudio_api.py"
    spec = importlib.util.spec_from_file_location("projectstudio_api", source)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"ProjectStudio API를 불러오지 못했습니다: {source}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def feature(feature_id: str, parent_id: str, title: str, description: str, role: str, sort_order: int, criteria: list[str]):
    return {
        "id": feature_id,
        "parentId": parent_id,
        "title": title,
        "description": description,
        "status": "done",
        "priority": "high",
        "role": role,
        "sortOrder": sort_order,
        "colorKey": "green",
        "acceptanceCriteria": [
            {"id": f"{feature_id}-ac-{index + 1}", "description": item, "isMet": True, "sortOrder": index}
            for index, item in enumerate(criteria)
        ],
    }


def desired_features():
    features = [
        feature(
            "feat-holdings-portfolio-analysis",
            "feat-meeting-evidence-pack",
            "전체 보유 포트폴리오 PIT 분석",
            "연결된 토스증권 보유종목을 임의 선택하지 않고 종목별 PIT 스냅샷과 익명화 포지션으로 한 회의에서 분석한다.",
            "리서치부 · 포트폴리오 관리자 · 데이터 품질 담당",
            634,
            [
                "복수 보유종목을 중복 제거하고 최대 20개까지 계좌 스냅샷의 구조화 종목코드로 순차 분석한다.",
                "종목별 공급자·기준 시각·통화·결측과 부분 실패를 보존하며 혼합 통화를 환율 없이 합산하지 않는다.",
                "계좌번호·별칭·자격정보를 AI에 전달하지 않고 수량·현재가·평단가만 근거 ID와 함께 전달한다.",
                "KRW·USD 현금 기반 매수 가능 금액을 계좌 식별자 없는 읽기 전용 근거로 전달한다.",
                "완료 일봉이 20개 미만인 종목은 가격·포지션을 유지하고 기술지표만 이력 부족으로 표시한다.",
                "다종목 결과는 사용자가 종목을 선택하기 전 백테스트·모의주문 후보로 직접 인계하지 않는다.",
            ],
        ),
        feature(
            "feat-user-portfolio-mandate",
            "req-risk",
            "사용자 운용 원칙 기반 집중도 판정",
            "집중도를 객관적 관측값과 사용자 활성 제약으로 분리해 기술주·테마주 집중을 앱이 임의로 위반 처리하지 않는다.",
            "포트폴리오 관리자 · 리스크관리 총괄",
            635,
            [
                "관측 전용·집중·테마·분산·사용자 정의 운용 원칙을 워크스페이스에 저장한다.",
                "사용자가 집중 한도를 활성화하지 않으면 종목·섹터·시장 비중을 감축·매도 판정 근거로 쓰지 않는다.",
                "명시적 한도가 있을 때만 실제 관측값과 비교하고, 일일 손실·낙폭·실주문 잠금은 별도 필수 안전 경계로 유지한다.",
                "구형 설정은 자동으로 관측 전용·집중 한도 비활성 상태로 안전하게 읽힌다.",
            ],
        ),
        feature(
            "feat-account-portfolio-presentation",
            "feat-holdings-portfolio-analysis",
            "실계좌 보유자산 기본 보기와 분석 기록 시각화",
            "원장 진입 시 읽기 전용 실계좌 보유자산을 먼저 표시하고 계좌 분석 기록에 당시의 익명화 포트폴리오 구성을 보존한다.",
            "프론트엔드 · 포트폴리오 관리자 · 보안 검토자",
            636,
            [
                "원장 기본 화면에서 실계좌 보유종목의 통화별 평가액·평가손익·비중·매수 가능 금액을 확인한다.",
                "실계좌 영역과 내부 모의원장·백테스트 원장을 읽기 전용 레이블과 정보 구조로 분리한다.",
                "계좌 분석 기록에는 계좌 식별자 없이 당시 포지션과 매수 가능 금액을 불변 스냅샷으로 저장한다.",
                "서로 다른 통화를 합산하지 않고 6개 초과 도넛 조각은 상위 5개와 기타로 묶되 상세 합계는 보존한다.",
                "업종·예상 배당 데이터가 없으면 그래프를 추정하지 않고 미제공 상태를 표시한다.",
            ],
        ),
        feature(
            "feat-telegram-evidence-autosync",
            "feat-meeting-evidence-pack",
            "Telegram 근거 자동 동기화와 진단",
            "분석 시작 시 사용자 선택 Telegram 채널을 동기화하고 연결·저장·포함 상태를 구분해 근거 묶음에 반영한다.",
            "뉴스 분석가 · 데이터 품질 담당 · 보안 검토자",
            637,
            [
                "Telegram 인증과 선택 채널이 있으면 분석 시작 시 읽기 전용 동기화를 수행한다.",
                "뉴스 cutoff는 완료 일봉 시각이 아니라 분석 요청 시작 시각으로 고정한다.",
                "동기화 실패는 분석 전체 실패로 숨기지 않고 근거 공백과 실패 사유로 기록한다.",
                "기간 내 저장 건수·보고 포함 건수·포함 채널 수·동기화 상태를 분석 기록에서 확인한다.",
                "Telegram 메시지의 명령과 링크는 실행하지 않고 허용 근거 ID만 보고 계약에 전달한다.",
            ],
        ),
    ]
    features.append({
        "id": "feat-complete-investment-analysis-report",
        "parentId": "feat-account-portfolio-presentation",
        "title": "10영역 투자 분석 보고 계약",
        "description": "최종 판단부터 데이터·가격대·기술·패턴·시나리오·대응·포트폴리오·생성물까지 실제 근거와 결측을 구분해 제공한다.",
        "status": "in_progress",
        "priority": "high",
        "role": "AI 투자본부장 · 리서치부 · 프론트엔드 · 데이터 품질 담당",
        "sortOrder": 638,
        "colorKey": "amber",
        "acceptanceCriteria": [
            {"id": "feat-complete-investment-analysis-report-ac-1", "description": "6단계 추천 등급과 기간별·신규·보유·추가매수 조건을 최종 종합에 포함한다.", "isMet": True, "sortOrder": 0},
            {"id": "feat-complete-investment-analysis-report-ac-2", "description": "OHLCV·출처·기준시각·SMA·RSI·ATR·MACD·Bollinger·최근 변동성과 데이터 실패 이유를 보존한다.", "isMet": True, "sortOrder": 1},
            {"id": "feat-complete-investment-analysis-report-ac-3", "description": "실제 완료 봉 차트에 MA5·20·60·거래량·지지·저항·추세선·박스 범위를 표시한다.", "isMet": True, "sortOrder": 2},
            {"id": "feat-complete-investment-analysis-report-ac-4", "description": "보유종목 분석 당시 포트폴리오와 종목별 주석 차트를 회의·부서 종합 기록에 저장한다.", "isMet": True, "sortOrder": 3},
            {"id": "feat-complete-investment-analysis-report-ac-5", "description": "반복 패턴 통계를 일반 분석 기록의 종목별 구조 필드로 제공한다.", "isMet": False, "sortOrder": 4},
            {"id": "feat-complete-investment-analysis-report-ac-6", "description": "차트 이미지 입력과 실제 OHLCV 비교를 출처가 구분된 구조로 저장한다.", "isMet": False, "sortOrder": 5},
            {"id": "feat-complete-investment-analysis-report-ac-7", "description": "상승·중립·하락 시나리오 경로 차트를 실제 데이터 기준 조건부 시각화로 제공한다.", "isMet": False, "sortOrder": 6},
            {"id": "feat-complete-investment-analysis-report-ac-8", "description": "카카오톡 전송용 단축 요약을 원 보고서와 근거 ID를 잃지 않는 파생 결과로 만든다.", "isMet": False, "sortOrder": 7},
        ],
    })
    return features


def normalize_flow(current):
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


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("projectstudio_root", type=Path)
    args = parser.parse_args()
    api = load_api(args.projectstudio_root)
    database = api.default_database_path()
    with api.connect_database(database, writable=True) as connection:
        current = api.get_project(connection, PROJECT_ID)
        desired = desired_features()
        desired_by_id = {item["id"]: item for item in desired}
        features = [desired_by_id.get(item["id"], item) for item in current["features"]]
        existing_ids = {item["id"] for item in features}
        features.extend(item for item in desired if item["id"] not in existing_ids)
        nodes, edges = normalize_flow(current)
        portfolio_node = {
            "id": FLOW_NODE_ID, "laneId": "lane-meeting-analysis-cycle", "title": "전체 보유종목 분석", "description": "복수 보유종목의 종목별 PIT 근거와 사용자 운용 원칙을 묶어 전체 포트폴리오를 분석한다.",
            "kind": "decision", "positionX": 710, "positionY": 4600, "colorKey": "green", "depth": None, "parentId": None,
            "linkedFeatureIds": sorted(FEATURE_IDS), "branchCondition": "보유종목 전체 분석 요청이며 2~20개 종목이 확인됨",
            "inputArtifacts": ["익명화 보유종목", "사용자 운용 원칙", "종목별 PIT 데이터"], "outputArtifacts": ["전체 포트폴리오 근거 묶음"],
            "methods": ["종목 중복 제거", "구조화 종목코드 순차 수집", "부분 실패 보존", "통화별 매수 가능 금액", "집중 판정 opt-in"],
            "validation": "모든 성공 종목의 근거 ID와 실패 종목 수가 일치하고 비활성 집중 한도가 판단에 쓰이지 않는다.",
            "failureHandling": "20개 초과는 일부 분석하지 않고 범위 축소를 요청하며 종목별 실패는 전체 성공으로 숨기지 않는다.",
            "codePaths": ["src/App.tsx", "src/meetingEvidence.ts", "src-tauri/src/operational_readiness.rs"],
            "testPaths": ["scripts/meetingEvidence.test.ts"],
            "completionCriteria": "전체 보유종목이 익명화된 종목별 근거로 분석되고 운용 원칙 없는 집중도는 관측값으로만 표시된다.", "isCompleted": True,
        }
        nodes = [portfolio_node if item["id"] == FLOW_NODE_ID else item for item in nodes]
        if not any(item["id"] == FLOW_NODE_ID for item in nodes):
            nodes.append(portfolio_node)
        presentation_node = {
            "id": PRESENTATION_NODE_ID, "laneId": "lane-meeting-analysis-cycle", "title": "보유자산 기록·원장 시각화", "description": "익명화 포트폴리오를 통화별 비중·손익으로 저장하고 원장 첫 화면에서 읽기 전용으로 표시한다.",
            "kind": "result", "positionX": 980, "positionY": 4600, "colorKey": "green", "depth": None, "parentId": None,
            "linkedFeatureIds": ["feat-account-portfolio-presentation", "feat-complete-investment-analysis-report"], "branchCondition": "실계좌 조회 또는 보유종목 분석 기록",
            "inputArtifacts": ["익명화 포지션", "통화별 매수 가능 금액"], "outputArtifacts": ["통화별 자산 구성", "분석 당시 포트폴리오 스냅샷"],
            "methods": ["통화 분리", "상위 5개+기타", "계좌 식별자 제거", "읽기 전용 레이블", "종목별 불변 주석 차트", "근거 충족도 진단"],
            "validation": "평가 합계·손익·비중 합계와 원본 포지션이 일치하고 계좌 식별자가 기록에 없다.",
            "failureHandling": "잘못된 숫자는 비중에서 제외하고 조회 실패는 모의원장을 가리지 않는 별도 빈 상태로 표시한다.",
            "codePaths": ["src/PortfolioOverview.tsx", "src/portfolioPresentation.ts", "src/LedgerWorkspace.tsx", "src/AnalysisWorkspace.tsx"],
            "testPaths": ["scripts/portfolioPresentation.test.ts", "scripts/meetingEvidence.test.ts"],
            "completionCriteria": "원장과 계좌 분석 기록에서 같은 익명화 포트폴리오 표현을 재현한다.", "isCompleted": True,
        }
        telegram_node = {
            "id": TELEGRAM_NODE_ID, "laneId": "lane-meeting-analysis-cycle", "title": "Telegram 근거 자동 동기화", "description": "선택 채널을 분석 시점에 동기화하고 최신 cutoff로 근거를 조립한다.",
            "kind": "action", "positionX": 980, "positionY": 4720, "colorKey": "cyan", "depth": None, "parentId": None,
            "linkedFeatureIds": ["feat-telegram-evidence-autosync"], "branchCondition": "Telegram 인증 완료 및 선택 채널 1개 이상",
            "inputArtifacts": ["사용자 선택 채널", "분석 요청 시작 시각", "분석 질의"], "outputArtifacts": ["Telegram PIT 근거", "동기화 진단"],
            "methods": ["선택 채널 읽기 동기화", "로컬 리비전 저장", "질의 관련도 정렬", "상태 분리"],
            "validation": "기간 내 저장·보고 포함·포함 채널 수가 기록되고 cutoff 이후 메시지가 제외된다.",
            "failureHandling": "인증·선택·동기화·기간 공백을 구분해 기록하고 시장·계좌 근거 분석은 계속한다.",
            "codePaths": ["src/App.tsx", "src-tauri/src/telegram.rs", "src-tauri/src/persistence.rs"],
            "testPaths": ["scripts/meetingEvidence.test.ts", "src-tauri/src/telegram.rs"],
            "completionCriteria": "연결된 Telegram 뉴스가 수동 새로고침 없이 현재 분석 근거 후보로 들어오고 상태가 설명된다.", "isCompleted": True,
        }
        desired_nodes = {PRESENTATION_NODE_ID: presentation_node, TELEGRAM_NODE_ID: telegram_node}
        nodes = [desired_nodes.get(item["id"], item) for item in nodes]
        node_ids = {item["id"] for item in nodes}
        nodes.extend(item for item in desired_nodes.values() if item["id"] not in node_ids)
        desired_edges = [
            {"id": "edge-meeting-symbol-portfolio", "sourceNodeId": "flow-meeting-analysis-cycle-symbol", "targetNodeId": FLOW_NODE_ID},
            {"id": "edge-meeting-portfolio-evidence", "sourceNodeId": FLOW_NODE_ID, "targetNodeId": "flow-meeting-analysis-cycle-evidence"},
            {"id": "edge-meeting-portfolio-presentation", "sourceNodeId": FLOW_NODE_ID, "targetNodeId": PRESENTATION_NODE_ID},
            {"id": "edge-meeting-portfolio-telegram", "sourceNodeId": FLOW_NODE_ID, "targetNodeId": TELEGRAM_NODE_ID},
            {"id": "edge-meeting-telegram-evidence", "sourceNodeId": TELEGRAM_NODE_ID, "targetNodeId": "flow-meeting-analysis-cycle-evidence"},
        ]
        edge_ids = {item["id"] for item in edges}
        edges.extend(item for item in desired_edges if item["id"] not in edge_ids)
        markdown = current["project"]["prd_markdown"]
        if SECTION_MARKER not in markdown:
            markdown += f"""

## {SECTION_MARKER} (2026-09-03)
- 전체 보유종목 요청은 한 종목을 임의 선택하지 않고 종목별 PIT 스냅샷과 익명화된 수량·현재가·평단가를 같은 회의 근거로 조립한다.
- 기술주·테마주 집중은 사용자의 운용 전략일 수 있으므로 집중도는 관측값으로 표시하되, 사용자가 집중 한도를 명시적으로 활성화한 경우에만 위반·감축 판단에 사용한다.
- 서로 다른 통화는 시점 정합 환율 없이 합산하지 않으며 부분 실패와 결측을 종목별로 표시한다. 다종목 보고는 사용자가 한 종목을 다시 선택하기 전 백테스트·모의주문으로 직접 넘기지 않는다.
- 미보유 후보를 찾는 종목 스크리너는 전체 보유 포트폴리오 진단과 별도 단계로 유지한다.
- 원장 첫 화면에는 실계좌 보유자산을 읽기 전용으로 먼저 표시하고 내부 모의원장과 분리한다. 계좌 분석 기록에는 당시 포트폴리오 비중·손익을 계좌 식별자 없이 보존한다.
- Telegram은 연결 여부만 보지 않고 분석 시작 시 선택 채널 동기화, 기간 내 저장, 현재 보고 포함 상태를 단계별로 확인한다. 뉴스 cutoff는 분석 요청 시작 시각이다.
- 분석 결과는 10개 출력 영역을 목표 계약으로 관리한다. 현재 완료된 포트폴리오·실제 OHLCV 주석 차트·기술지표·근거 진단과, 남은 반복 패턴 일반화·이미지 비교·시나리오 차트·단축 요약을 구분한다.
"""
        if PRESENTATION_MARKER not in markdown:
            markdown += f"""

## {PRESENTATION_MARKER} (2026-09-03)
- 원장 진입 시 실계좌 보유자산을 읽기 전용 구역으로 먼저 표시하고 내부 모의원장·백테스트 원장과 명확히 분리한다.
- 계좌 분석 기록에는 당시의 익명화 포지션과 통화별 매수 가능 금액을 저장해 통화별 평가액·평가손익·비중을 재현한다. 계좌 별칭과 번호는 기록하지 않는다.
- 서로 다른 통화를 환율 없이 합산하지 않으며 업종·예상 배당 데이터가 없으면 추정 그래프를 만들지 않는다.
- Telegram은 분석 시작 시 사용자 선택 채널을 동기화하고 `연결`, `선택`, `기간 내 저장`, `보고 포함` 상태를 구분한다. 완료 일봉 시각이 아니라 분석 요청 시작 시각을 뉴스 cutoff로 사용한다.
"""
        if OUTPUT_CONTRACT_MARKER not in markdown:
            markdown += f"""

## {OUTPUT_CONTRACT_MARKER} (2026-09-04)
- 최종 판단, 데이터 확인, 핵심 가격대, 기술 분석, 반복 패턴, 이미지 비교, 3개 시나리오, 대상별 대응, 포트폴리오 위험과 생성 결과물을 하나의 목표 계약으로 관리한다.
- 보유종목 분석 당시 익명화 포트폴리오와 종목별 완료 OHLCV·MA5·20·60·거래량·지지·저항·추세선·가격 범위를 회의와 승인형 부서 기록에 함께 보존한다.
- 근거 충족도는 방향 확률이 아닌 부서 자체평가로 표시하고 고유 근거 ID, 근거가 있는 직원과 남은 공백을 함께 제시한다.
- 현재 미완료인 일반 분석 반복 패턴 구조화, 이미지 대 실제 데이터 비교, 조건부 시나리오 차트와 카카오톡 단축 요약은 완료로 체크하지 않는다.
"""
        current_feature_by_id = {item["id"]: item for item in current["features"]}
        existing_node = next((item for item in normalize_flow(current)[0] if item["id"] == FLOW_NODE_ID), None)
        existing_nodes = {item["id"]: item for item in normalize_flow(current)[0]}
        existing_edge_ids = {item["id"] for item in normalize_flow(current)[1]}
        if (
            all(current_feature_by_id.get(item["id"]) == item for item in desired)
            and existing_node == portfolio_node
            and all(existing_nodes.get(node_id) == node for node_id, node in desired_nodes.items())
            and all(item["id"] in existing_edge_ids for item in desired_edges)
            and SECTION_MARKER in current["project"]["prd_markdown"]
            and PRESENTATION_MARKER in current["project"]["prd_markdown"]
            and OUTPUT_CONTRACT_MARKER in current["project"]["prd_markdown"]
        ):
            print(json.dumps({"projectId": PROJECT_ID, "committed": False, "message": "동일한 포트폴리오 분석 기획이 이미 반영되어 있습니다."}, ensure_ascii=False, indent=2))
            return
        bundle = {
            "schemaVersion": 1, "projectId": PROJECT_ID, "expectedPrdRevisionNumber": current["project"]["revision_number"],
            "prd": {"title": current["project"]["prd_title"], "markdown": markdown}, "features": features, "userFlow": {"nodes": nodes, "edges": edges},
        }
        api.validate_bundle(bundle)
        print(json.dumps(api.apply_bundle(connection, database, bundle, commit=True), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
