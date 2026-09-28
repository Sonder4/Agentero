import { describe, expect, it, vi } from "vitest";

const { currentWebview, eventTarget } = vi.hoisted(() => ({
	currentWebview: { label: "embedded-agentero" },
	eventTarget: { value: undefined as unknown },
}));

vi.mock("@tauri-apps/api/webview", () => ({
	getCurrentWebview: vi.fn(() => currentWebview),
}));

// Before the fix this mock represented the call made by api.ts. A nested
// Tauri Webview is not a WebviewWindow, so the corresponding event IPC rejects
// with `current webview is not a WebviewWindow`.
vi.mock("@tauri-apps/api/webviewWindow", () => ({
	getCurrentWebviewWindow: vi.fn(() => {
		throw new Error("current webview is not a WebviewWindow");
	}),
}));

vi.mock("@/lib/core/bindings", () => ({
	commands: {},
	events: {
		agentCompleted: (target: unknown) => {
			eventTarget.value = target;
			return { listen: () => Promise.resolve(() => undefined) };
		},
	},
}));

describe("Agent event listeners", () => {
	it("targets the current Webview so nested webviews can receive ACP events", async () => {
		const { listenAgentCompleted } = await import("@/lib/agent/api");
		const unlisten = await listenAgentCompleted(() => undefined);

		expect(eventTarget.value).toBe(currentWebview);
		expect(unlisten).toEqual(expect.any(Function));
	});
});
