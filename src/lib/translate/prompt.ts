/**
 * Prompts for the Agent translation provider.
 * Generic surface + PDF selection variant.
 */

/** True when `text` is a numbered batch payload (`[[1]] …`, `[[2]] …`). */
function hasNumberedMarkers(text: string): boolean {
	return /\[{2}\s*\d+\s*\]{2}/.test(text);
}

/**
 * Built-in instruction block (role + direction + rules) with `{{targetLang}}`
 * placeholders. Also the seed filled in by Settings → 翻译 → 翻译提示词; a
 * non-empty `translate.customPrompt` replaces it wholesale.
 */
export const DEFAULT_TRANSLATE_PROMPT_TEMPLATE = `You are a professional academic translator working inside Agentero, a research paper workbench.

Translate the text below into {{targetLang}}.

Rules:
- The source is prose from a research paper, often extracted from a PDF text layer. Translate the meaning, not the word order: write natural, fluent {{targetLang}} the way a researcher in the field would. Re-order clauses and split long sentences when that reads better.
- Keep mathematics, symbols, variable names, units, inline code, URLs and citation markers ([12], (Smith et al., 2020)) exactly as they appear, including any ⟦n⟧ placeholders.
- Keep figure / table / section / equation numbers unchanged.
- Use the established {{targetLang}} term for each concept and stay consistent; on a term's first occurrence, follow it with the original in parentheses, e.g. 注意力机制（attention）.
- Do not add, drop, summarize or explain anything. No translator notes, no extra headings, no markdown fences.
- Output only the translation.`;

/**
 * Substitute `{{targetLang}}` / `{{sourceLang}}` in a prompt template. The
 * frontend source language is always auto-detected, so `{{sourceLang}}`
 * renders the same wording the built-in prompt uses.
 */
export function renderTranslatePromptTemplate(
	template: string,
	targetLangName: string,
): string {
	return template
		.replaceAll("{{targetLang}}", targetLangName)
		.replaceAll("{{sourceLang}}", "the source language");
}

export function buildTranslatePrompt(opts: {
	text: string;
	targetLangName: string;
	page?: number;
	surface?: string;
	/** Non-empty replaces the default instruction block (role + rules). */
	customPrompt?: string;
	context?: {
		previousParagraph?: string;
		nextParagraph?: string;
		previousTranslatedExcerpt?: string;
		relatedFormulas?: readonly string[];
		relatedTables?: readonly string[];
		relatedFigures?: readonly string[];
		glossary?: readonly {
			source: string;
			aliases?: readonly string[];
			target: string;
		}[];
	};
}): string {
	const text = opts.text.trim();
	const lang = opts.targetLangName;
	const custom = opts.customPrompt?.trim();
	const template = custom || DEFAULT_TRANSLATE_PROMPT_TEMPLATE;
	const parts = [renderTranslatePromptTemplate(template, lang)];
	if (opts.surface === "pdf-selection" && opts.page != null) {
		parts.push(`Source: research paper PDF, page ${opts.page}.`);
	}
	if (hasNumberedMarkers(text)) {
		parts.push(
			"The text contains several paragraphs, each prefixed with a [[n]] marker. " +
				"Translate every paragraph and keep the same [[n]] markers, in the same " +
				"order, with the same number of paragraphs. Do not merge paragraphs.",
		);
	}
	const context = opts.context;
	if (context?.glossary?.length) {
		const rows = context.glossary
			.map(
				(term) =>
					`${term.source} | ${(term.aliases ?? []).join(", ")} | ${term.target}`,
			)
			.join("\n");
		parts.push(
			"Object-scoped glossary (use these translations consistently; do not translate the table itself):\n" +
				"Source | Aliases | Target\n" +
				rows,
		);
	}
	const contextParts: string[] = [];
	if (context?.previousParagraph?.trim()) {
		contextParts.push(
			`Previous paragraph (read-only):\n${context.previousParagraph.trim()}`,
		);
	}
	if (context?.nextParagraph?.trim()) {
		contextParts.push(
			`Next paragraph (read-only):\n${context.nextParagraph.trim()}`,
		);
	}
	if (context?.previousTranslatedExcerpt?.trim()) {
		contextParts.push(
			`Previous translated excerpt (read-only):\n${context.previousTranslatedExcerpt.trim()}`,
		);
	}
	if (context?.relatedFormulas?.length) {
		contextParts.push(
			`Related formulas (preserve exactly):\n${context.relatedFormulas.join("\n")}`,
		);
	}
	if (context?.relatedTables?.length) {
		contextParts.push(
			`Related tables (read-only):\n${context.relatedTables.join("\n")}`,
		);
	}
	if (context?.relatedFigures?.length) {
		contextParts.push(
			`Related figures/captions (read-only):\n${context.relatedFigures.join("\n")}`,
		);
	}
	if (contextParts.length) {
		parts.push(
			"The following context belongs to the same translation object. Use it only for terminology and reference resolution; do not translate or copy it into the answer.\n\n" +
				contextParts.join("\n\n"),
		);
	}
	parts.push("Text:", text);
	return parts.join("\n\n");
}
