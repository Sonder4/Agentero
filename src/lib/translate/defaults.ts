import type {
	TranslateProviderConfig,
	TranslateSettings,
} from "@/lib/translate/types";

export const DEFAULT_LAYOUT_TRANSLATE_CONCURRENCY = 2;
export const LAYOUT_TRANSLATE_CONCURRENCY_MIN = 1;
export const LAYOUT_TRANSLATE_CONCURRENCY_MAX = 8;
export const LAYOUT_TRANSLATE_CONCURRENCY_OPTIONS = [1, 2, 3, 4, 6, 8] as const;

/** Normalize hand-edited or legacy values to a usable worker count. */
export function clampLayoutTranslateConcurrency(value: number): number {
	if (!Number.isFinite(value)) return DEFAULT_LAYOUT_TRANSLATE_CONCURRENCY;
	return Math.min(
		LAYOUT_TRANSLATE_CONCURRENCY_MAX,
		Math.max(LAYOUT_TRANSLATE_CONCURRENCY_MIN, Math.round(value)),
	);
}

export const DEFAULT_TRANSLATE_SETTINGS: TranslateSettings = {
	/** Prefer Tencent Transmart: current no-key default with better availability. */
	provider: "tencenttransmart",
	targetLang: "ui",
	sourceLang: "auto",
	providerConfigs: {},
	autoTranslateSelection: false,
	dualPaneTranslate: false,
	layoutTranslateConcurrency: DEFAULT_LAYOUT_TRANSLATE_CONCURRENCY,
	displayMode: "overlay",
	dualPaneSource: "pdf",
	agentId: "",
	modelId: "",
	customPrompt: "",
};

/** Blank commercial provider config (missing draft/stored entry fallback). */
export const EMPTY_TRANSLATE_PROVIDER_CONFIG: TranslateProviderConfig = {
	apiKey: "",
	baseUrl: "",
	region: "",
	model: "",
};
