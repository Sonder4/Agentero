import { beforeEach, describe, expect, it, vi } from "vitest";

const completedListeners: Array<(event: Record<string, unknown>) => void> = [];
const failedListeners: Array<(event: Record<string, unknown>) => void> = [];
const runOnce = vi.fn();
const cancelAgentRun = vi.fn(async () => undefined);

vi.mock("@/lib/agent", () => ({
	runOnce: (...args: unknown[]) => runOnce(...args),
	cancelAgentRun: (...args: unknown[]) => cancelAgentRun(...args),
	listenAgentCompleted: (handler: (event: Record<string, unknown>) => void) => {
		completedListeners.push(handler);
		return Promise.resolve(() => undefined);
	},
	listenAgentFailed: (handler: (event: Record<string, unknown>) => void) => {
		failedListeners.push(handler);
		return Promise.resolve(() => undefined);
	},
}));

vi.mock("@/lib/settings", () => ({
	loadSettings: () => ({
		translate: {
			provider: "agent",
			targetLang: "zh-CN",
			sourceLang: "auto",
			providerConfigs: {},
			layoutTranslateConcurrency: 1,
			agentId: "agent-1",
			modelId: "model-1",
			customPrompt: "",
		},
	}),
}));

vi.mock("@/lib/translate/resolve-agent", () => ({
	resolveConfiguredTranslateAgent: async () => ({
		agentId: "agent-1",
		modelId: "model-1",
	}),
}));

vi.mock("@/lib/pdf/translate/agent-session-cache", () => ({
	getAgentTranslateSessionId: () => undefined,
	setAgentTranslateSessionId: vi.fn(),
	evictAgentTranslateSessionId: vi.fn(),
}));

vi.mock("@/lib/pdf/layout/layout-translate-object", () => ({
	readLayoutTranslateGlossary: async () => ({ terms: [], contentHash: "" }),
}));

import { runLayoutRegionTranslate } from "@/lib/pdf/layout/layout-translate";
import type { LayoutTranslateItem } from "@/lib/pdf/layout/types";

function item(): LayoutTranslateItem {
	return {
		id: "a",
		pageIndex: 0,
		bbox: { x: 0, y: 0, w: 1, h: 1 },
		kind: "text",
		readingOrder: 0,
		source: "Hello",
		status: "pending",
	};
}

describe("layout translation agent lifecycle", () => {
	beforeEach(() => {
		runOnce.mockReset();
		cancelAgentRun.mockClear();
		completedListeners.length = 0;
		failedListeners.length = 0;
	});

	it("captures a completion emitted synchronously by runOnce", async () => {
		runOnce.mockImplementation(async () => {
			completedListeners[0]?.({
				sessionId: "session-1",
				content: "你好",
				stopReason: "end_turn",
			});
			return {
				sessionId: "session-1",
				messageId: "message-1",
				agentId: "agent-1",
			};
		});

		const result = await runLayoutRegionTranslate({
			items: [item()],
			onUpdate: () => {},
		});
		expect(result[0]?.translated).toBe("你好");
		expect(result[0]?.status).toBe("done");
	});

	it("cancels the accepted agent session when aborted", async () => {
		const controller = new AbortController();
		runOnce.mockResolvedValue({
			sessionId: "session-2",
			messageId: "message-2",
			agentId: "agent-1",
		});
		const pending = runLayoutRegionTranslate({
			items: [item()],
			signal: controller.signal,
			onUpdate: () => {},
		});
		await vi.waitFor(() => expect(runOnce).toHaveBeenCalledTimes(1));
		controller.abort();
		const result = await pending;
		expect(cancelAgentRun).toHaveBeenCalledWith("session-2");
		expect(result[0]?.status).toBe("skipped");
	});
});
