import assert from "node:assert/strict";
import test from "node:test";

import { CONNECTION_LEGEND, connectionTone } from "../src/connectionStatus.ts";

test("실제 연결 성공만 연결 완료 색상으로 분류한다", () => {
  assert.equal(connectionTone(true, true), "connected");
  assert.equal(connectionTone(true, false), "connected");
});

test("저장만 된 연결과 미연결을 구분한다", () => {
  assert.equal(connectionTone(false, true), "partial");
  assert.equal(connectionTone(false, false), "disconnected");
});

test("연결 상태 범례는 청록·주황·회색의 의미를 모두 제공한다", () => {
  assert.deepEqual(CONNECTION_LEGEND.map(({ tone, label }) => ({ tone, label })), [
    { tone: "connected", label: "연결 완료" },
    { tone: "partial", label: "확인 필요" },
    { tone: "disconnected", label: "미연결" },
  ]);
});
