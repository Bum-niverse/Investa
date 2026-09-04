"""분석 근거 가용성 게이트를 ProjectStudio에 멱등 반영한다."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path


PROJECT_ID = "36e87491-74a8-48ca-a7b8-30fa6ccea131"
FEATURE_ID = "feat-analysis-evidence-availability-gate"
FLOW_NODE_ID = "flow-analysis-evidence-availability-gate"
SECTION_MARKER = "분석 근거 가용성 게이트와 공급자 계약 분리"


def load_api(root: Path):
    source = root / "scripts" / "projectstudio_api.py"
    spec = importlib.util.spec_from_file_location("projectstudio_api", source)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"ProjectStudio API를 불러오지 못했습니다: {source}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def normalize_flow(current):
    nodes = []
    for item in current["userFlow"]["nodes"]:
        metadata = json.loads(item.get("metadata_json") or "{}")
        nodes.append({
            "id": item["id"], "laneId": item["lane_id"], "title": item["title"], "description": item["description"], "kind": item["kind"],
            "positionX": item["position_x"], "positionY": item["position_y"], "colorKey": item.get("color_key") or "violet", "depth": item.get("depth"),
            "parentId": item.get("parent_id"), "linkedFeatureIds": json.loads(item.get("linked_feature_ids") or "[]"), "branchCondition": item.get("branch_condition"),
            "inputArtifacts": metadata.get("inputArtifacts", []), "outputArtifacts": metadata.get("outputArtifacts", []), "methods": metadata.get("methods", []),
            "validation": metadata.get("validation", ""), "failureHandling": metadata.get("failureHandling", ""), "codePaths": metadata.get("codePaths", []),
            "testPaths": metadata.get("testPaths", []), "completionCriteria": metadata.get("completionCriteria", ""), "isCompleted": bool(metadata.get("isCompleted", False)),
        })
    edges = [{"id": item["id"], "sourceNodeId": item["source_node_id"], "targetNodeId": item["target_node_id"]} for item in current["userFlow"]["edges"]]
    return nodes, edges


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("projectstudio_root", type=Path)
    args = parser.parse_args()
    api = load_api(args.projectstudio_root)
    database = api.default_database_path()
    criteria = [
        "기술·재무·뉴스 담당의 최소 읽기 전용 근거 도구는 Agent 선택 누락과 무관하게 자동 실행한다.",
        "유한 MACD·Bollinger와 지지·저항·추세선·가격 범위 주석을 직원과 부서장에게 전달한다.",
        "프로그램 가용성 매니페스트에서 1건 이상인 근거는 역할 밖 직원 문장으로 결측 처리하지 않는다.",
        "TOSS_OPEN_API 자료에는 TOSS 계약만, KIS 자료에는 KIS 계약만 검증한다.",
        "실제 0건과 짧은 상장 이력은 원인과 함께 결측으로 유지한다.",
    ]
    feature = {
        "id": FEATURE_ID, "parentId": "feat-complete-investment-analysis-report", "title": "분석 근거 가용성 게이트",
        "description": "실행 시점의 실제 지표·차트·재무·공시·뉴스 근거와 공급자 계약을 결정론적으로 판정해 Agent의 잘못된 결측 확대를 차단한다.",
        "status": "done", "priority": "high", "role": "리서치부 · 데이터 품질 담당 · 보안 검토자", "sortOrder": 640, "colorKey": "green",
        "acceptanceCriteria": [{"id": f"{FEATURE_ID}-ac-{i + 1}", "description": value, "isMet": True, "sortOrder": i} for i, value in enumerate(criteria)],
    }
    with api.connect_database(database, writable=True) as connection:
        current = api.get_project(connection, PROJECT_ID)
        features = [feature if item["id"] == FEATURE_ID else item for item in current["features"]]
        if not any(item["id"] == FEATURE_ID for item in features):
            features.append(feature)
        nodes, edges = normalize_flow(current)
        node = {
            "id": FLOW_NODE_ID, "laneId": "lane-meeting-analysis-cycle", "title": "근거 가용성 게이트", "description": "도구 실행 결과를 LLM 서술과 분리해 공급자별 근거 존재 여부를 확정한다.",
            "kind": "action", "positionX": 1240, "positionY": 4720, "colorKey": "green", "depth": None, "parentId": None,
            "linkedFeatureIds": [FEATURE_ID, "feat-complete-investment-analysis-report"], "branchCondition": "직원별 읽기 전용 도구 실행 완료",
            "inputArtifacts": ["기술 스냅샷", "차트 주석", "직원 근거 메타데이터", "Telegram 동기화 상태"], "outputArtifacts": ["공급자별 가용성 매니페스트"],
            "methods": ["필수 도구 자동 보완", "유한 지표 판정", "근거 ID 중복 제거", "공급자 계약 분리"],
            "validation": "TOSS 기술·Telegram 근거가 존재하는 회귀 입력에서 KIS 미검증 또는 전체 뉴스 부재를 만들지 않는다.",
            "failureHandling": "실제 0건인 개별 항목만 결측으로 남기고 다른 근거의 존재를 지우지 않는다.",
            "codePaths": ["src/App.tsx", "src/employeeAgentOrchestration.ts", "src/technicalChartEvidence.ts"],
            "testPaths": ["scripts/employeeAgentOrchestration.test.ts", "scripts/technicalChartEvidence.test.ts"],
            "completionCriteria": "가용성 매니페스트와 직원 보고가 충돌할 때 프로그램 관측값을 우선하고 역할 밖 결측을 확대하지 않는다.", "isCompleted": True,
        }
        nodes = [node if item["id"] == FLOW_NODE_ID else item for item in nodes]
        if not any(item["id"] == FLOW_NODE_ID for item in nodes):
            nodes.append(node)
        edge = {"id": "edge-staged-synthesis-evidence-availability", "sourceNodeId": "flow-meeting-staged-synthesis", "targetNodeId": FLOW_NODE_ID}
        if not any(item["id"] == edge["id"] for item in edges):
            edges.append(edge)
        markdown = current["project"]["prd_markdown"]
        if SECTION_MARKER not in markdown:
            markdown += f"""

## {SECTION_MARKER} (2026-09-04)
- 기술·재무·뉴스 담당의 최소 읽기 전용 도구를 자동 보완하고 결과를 프로그램 가용성 매니페스트로 고정한다.
- 유한 MACD·Bollinger 값과 지지·저항·추세선·가격 범위 주석, 재무·공시·일반 뉴스·Telegram 건수를 부서장에게 전달한다.
- 직원의 역할 밖 결측 문장은 존재하는 근거를 지울 수 없으며 TOSS 자료에 KIS 계약을 요구하지 않는다.
- 실제 공급자 0건과 신규 상장 이력 부족은 원인을 보존하며 실주문은 계속 금지한다.
"""
        existing = next((item for item in current["features"] if item["id"] == FEATURE_ID), None)
        existing_nodes = {item["id"]: item for item in normalize_flow(current)[0]}
        if existing == feature and existing_nodes.get(FLOW_NODE_ID) == node and SECTION_MARKER in current["project"]["prd_markdown"]:
            print(json.dumps({"projectId": PROJECT_ID, "committed": False, "message": "동일한 근거 가용성 기획이 이미 반영되어 있습니다."}, ensure_ascii=False, indent=2))
            return
        bundle = {
            "schemaVersion": 1, "projectId": PROJECT_ID, "expectedPrdRevisionNumber": current["project"]["revision_number"],
            "prd": {"title": current["project"]["prd_title"], "markdown": markdown}, "features": features, "userFlow": {"nodes": nodes, "edges": edges},
        }
        api.validate_bundle(bundle)
        print(json.dumps(api.apply_bundle(connection, database, bundle, commit=True), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
