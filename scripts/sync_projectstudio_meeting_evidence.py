"""회의 근거 묶음 구현을 ProjectStudio Investa 기능명세에 멱등 반영한다."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path


PROJECT_ID = "36e87491-74a8-48ca-a7b8-30fa6ccea131"
FEATURE_ID = "feat-meeting-evidence-pack"
FLOW_NODE_ID = "flow-meeting-analysis-cycle-3"
SECTION_MARKER = "부서장 회의 근거 묶음"


def load_api(root: Path):
    source = root / "scripts" / "projectstudio_api.py"
    spec = importlib.util.spec_from_file_location("projectstudio_api", source)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"ProjectStudio API를 불러오지 못했습니다: {source}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def desired_feature():
    criteria = [
        ("symbol-position", "안건의 단일 종목 스냅샷과 같은 종목의 토스 보유 수량·평단가·현재가만 익명화해 연결한다."),
        ("evidence-ids", "가격·기술·포지션·Telegram 자료에 결정론적 근거 ID와 관측 시각을 부여해 부서 보고와 본부장 종합에 보존한다."),
        ("partial-gap", "국장 재무·공시처럼 미연결인 공급자는 해당 항목만 근거 공백으로 남기고 사용 가능한 근거까지 전체 공백으로 만들지 않는다."),
        ("privacy-boundary", "전체 계좌번호·계좌 별칭·자격정보는 Codex 요청과 로그에 포함하지 않는다."),
        ("shadow-boundary", "SHADOW ONLY에서 내부 모의주문 후보 검토는 허용하되 실주문은 항상 금지한다."),
    ]
    return {
        "id": FEATURE_ID,
        "parentId": "feat-agents-trace",
        "title": "부서장 회의 데이터 근거 묶음",
        "description": "연결된 시장·계좌·Telegram 데이터를 하나의 기준 시각 근거 묶음으로 조립해 모든 소집 부서와 본부장 종합에 전달한다.",
        "status": "done",
        "priority": "high",
        "role": "시장데이터 엔지니어 · 데이터 품질 담당 · 보안 담당",
        "sortOrder": 633,
        "colorKey": "green",
        "acceptanceCriteria": [
            {"id": f"ac-meeting-evidence-{suffix}", "description": description, "isMet": True, "sortOrder": index}
            for index, (suffix, description) in enumerate(criteria)
        ],
    }


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
        desired = desired_feature()
        features = [desired if item["id"] == FEATURE_ID else item for item in current["features"]]
        if not any(item["id"] == FEATURE_ID for item in features):
            features.append(desired)

        nodes, edges = normalize_flow(current)
        node = next(item for item in nodes if item["id"] == FLOW_NODE_ID)
        node["linkedFeatureIds"] = list(dict.fromkeys([*node["linkedFeatureIds"], FEATURE_ID]))
        node["inputArtifacts"] = ["회의 안건", "PIT 분석 스냅샷", "익명화 포지션", "선택 Telegram 근거"]
        node["outputArtifacts"] = ["근거 ID가 포함된 부서 보고"]
        node["methods"] = ["단일 종목 확정", "기준 시각 고정", "민감 계좌 식별정보 제거", "부분 결측 보존"]
        node["validation"] = "가격·기술·포지션·뉴스 근거 ID가 부서 보고와 본부장 종합에서 추적되고 실주문은 잠겨 있다."
        node["failureHandling"] = "종목 미확정 또는 공급자 오류는 해당 근거만 공백으로 남기며 존재하지 않는 수치와 ID를 만들지 않는다."
        node["codePaths"] = ["src/App.tsx", "src/meetingEvidence.ts", "src-tauri/src/market_data.rs"]
        node["testPaths"] = ["scripts/meetingEvidence.test.ts"]
        node["completionCriteria"] = "연결된 토스 포지션과 시장·Telegram 근거가 익명화된 동일 회의 근거 묶음으로 전달된다."

        markdown = current["project"]["prd_markdown"]
        if SECTION_MARKER not in markdown:
            markdown += f"""

## {SECTION_MARKER} (2026-08-30)
- 회의 분류 뒤 단일 기준 시각의 시장 스냅샷, 해당 종목의 익명화된 토스 보유 포지션과 선택 Telegram 자료를 한 번 수집해 모든 소집 부서가 재사용한다.
- 가격·기술·포지션·Telegram 자료에는 결정론적 근거 ID를 부여하며 본부장 종합에도 해당 ID와 항목별 근거 공백을 유지한다.
- 전체 계좌번호·계좌 별칭·자격정보는 Codex 요청에 넣지 않는다. 국장 재무·공시는 OpenDART 어댑터가 연결될 때까지 그 항목만 결측이다.
- SHADOW ONLY는 내부 모의주문 후보 검토를 허용하지만 실주문 전송은 계속 금지한다.
"""

        existing = next((item for item in current["features"] if item["id"] == FEATURE_ID), None)
        already_equal = existing == desired and FEATURE_ID in node["linkedFeatureIds"] and SECTION_MARKER in current["project"]["prd_markdown"]
        if already_equal:
            print(json.dumps({"projectId": PROJECT_ID, "committed": False, "message": "동일한 회의 근거 묶음 기획이 이미 반영되어 있습니다."}, ensure_ascii=False, indent=2))
            return
        bundle = {
            "schemaVersion": 1,
            "projectId": PROJECT_ID,
            "expectedPrdRevisionNumber": current["project"]["revision_number"],
            "prd": {"title": current["project"]["prd_title"], "markdown": markdown},
            "features": features,
            "userFlow": {"nodes": nodes, "edges": edges},
        }
        api.validate_bundle(bundle)
        print(json.dumps(api.apply_bundle(connection, database, bundle, commit=True), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
