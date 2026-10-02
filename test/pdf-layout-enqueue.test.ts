import { beforeEach, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	enqueue: vi.fn(),
	readSidecar: vi.fn(),
	vaultPath: "\\\\?\\E:\\vault",
}));
vi.mock("@/i18n", () => ({ default: { t: (key: string) => key } }));
vi.mock("@/lib/core/bindings", () => ({
	commands: { jobLayoutAnalyzeEnqueue: mocks.enqueue },
}));
vi.mock("@/lib/core/ipc", () => ({
	callApiResult: (fn: () => unknown) => fn(),
}));
vi.mock("@/lib/core/logger", () => ({ logger: { warn: vi.fn() } }));
vi.mock("@/lib/core/tasks", () => ({ registerTaskExecutor: vi.fn() }));
vi.mock("@/lib/pdf/layout/headless-analyze", () => ({
	analyzePaperLayoutHeadless: vi.fn(),
}));
vi.mock("@/lib/pdf/layout/io", () => ({
	readLayoutSidecar: mocks.readSidecar,
}));
vi.mock("@/lib/vault/store", () => ({ getVaultPath: () => mocks.vaultPath }));

import { enqueuePaperLayoutAnalysis } from "@/lib/pdf/layout/enqueue-paper-layout";

beforeEach(() => {
	mocks.enqueue.mockReset();
	mocks.readSidecar.mockReset().mockResolvedValue(null);
});

it("preserves native extended Windows paths for IO and sends a relative job path", async () => {
	mocks.enqueue.mockResolvedValue({ id: "job-1" });
	const paperAbsPath = "\\\\?\\E:\\vault\\papers\\adam";
	enqueuePaperLayoutAnalysis({ paperAbsPath });
	await vi.waitFor(() => expect(mocks.enqueue).toHaveBeenCalledOnce());
	expect(mocks.readSidecar).toHaveBeenCalledWith(paperAbsPath);
	expect(mocks.enqueue).toHaveBeenCalledWith({
		vaultPath: mocks.vaultPath,
		path: "papers/adam",
		lane: "normal",
		force: false,
	});
});

it("notifies a translation waiting on an already pending automatic enqueue", async () => {
	let rejectRead!: (error: unknown) => void;
	mocks.readSidecar.mockReturnValue(
		new Promise((_, reject) => {
			rejectRead = reject;
		}),
	);
	const onError = vi.fn();
	enqueuePaperLayoutAnalysis({
		paperAbsPath: "\\\\?\\E:\\vault\\papers\\retry",
	});
	enqueuePaperLayoutAnalysis({
		paperAbsPath: "//?/E:/vault/papers/retry",
		onError,
	});
	expect(mocks.readSidecar).toHaveBeenCalledOnce();
	const error = new Error("paper folder not found");
	rejectRead(error);
	await vi.waitFor(() => expect(onError).toHaveBeenCalledWith(error));
	mocks.readSidecar.mockResolvedValue(null);
	mocks.enqueue.mockResolvedValue({ id: "retry-job" });
	enqueuePaperLayoutAnalysis({
		paperAbsPath: "\\\\?\\E:\\vault\\papers\\retry",
		onError,
	});
	await vi.waitFor(() => expect(mocks.enqueue).toHaveBeenCalledOnce());
});
