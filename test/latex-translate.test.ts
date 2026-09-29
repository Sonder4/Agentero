import { describe, expect, it } from "vitest";

import {
	insertCommandArgSeparators,
	translateLatexBody,
	translateLatexContent,
} from "@/lib/translate/latex-translate";

const identityTranslator = async (text: string): Promise<string> => text;

describe("insertCommandArgSeparators", () => {
	it("does not split command names followed by arguments", () => {
		expect(insertCommandArgSeparators("\\documentclass[sigconf]{acmart}")).toBe(
			"\\documentclass[sigconf]{acmart}",
		);
		expect(insertCommandArgSeparators("\\usepackage[utf8]{inputenc}")).toBe(
			"\\usepackage[utf8]{inputenc}",
		);
		expect(insertCommandArgSeparators("\\textbf{bold text}")).toBe(
			"\\textbf{bold text}",
		);
	});

	it("inserts {} between a command and a following CJK character", () => {
		expect(insertCommandArgSeparators("\\LaTeX中文")).toBe("\\LaTeX{}中文");
		expect(insertCommandArgSeparators("\\emph中文文本")).toBe(
			"\\emph{}中文文本",
		);
	});

	it("does not insert {} before ASCII letters (avoids splitting command names)", () => {
		// \LaTeXis could be \LaTeX followed by "is" or a single command \LaTeXis;
		// we cannot tell safely, so we leave it to LaTeX's command-name parser.
		expect(insertCommandArgSeparators("\\LaTeXis great")).toBe(
			"\\LaTeXis great",
		);
	});

	it("does not insert {} when the command is already terminated by space or {}", () => {
		expect(insertCommandArgSeparators("\\LaTeX is great")).toBe(
			"\\LaTeX is great",
		);
		expect(insertCommandArgSeparators("\\LaTeX{}中文")).toBe("\\LaTeX{}中文");
	});

	it("leaves single-character escapes alone", () => {
		expect(insertCommandArgSeparators("\\% \\, \\{ \\}")).toBe(
			"\\% \\, \\{ \\}",
		);
		expect(insertCommandArgSeparators("\\\\ newline")).toBe("\\\\ newline");
	});

	it("handles multiple commands in one line", () => {
		expect(
			insertCommandArgSeparators("\\documentclass[sigconf]{acmart}\\LaTeX中文"),
		).toBe("\\documentclass[sigconf]{acmart}\\LaTeX{}中文");
	});
});

describe("translateLatexContent pipeline", () => {
	it("keeps command names with arguments intact", async () => {
		const text = "\\documentclass[sigconf]{acmart}\n\nWe use ACM style.";
		const result = await translateLatexContent(text, identityTranslator);
		expect(result).toContain("\\documentclass[sigconf]{acmart}");
		expect(result).not.toContain("\\documentclas{}s");
	});

	it("inserts {} between a command and a following CJK character", async () => {
		const result = await translateLatexContent(
			"\\LaTeX中文",
			identityTranslator,
		);
		expect(result).toContain("\\LaTeX{}中文");
	});
});

describe("translateLatexBody progress", () => {
	it("reports monotonic progress through nested environments", async () => {
		const text = [
			"First introduction paragraph.",
			"",
			"Second introduction paragraph.",
			"",
			"\\begin{abstract}",
			"Abstract paragraph one.",
			"",
			"Abstract paragraph two.",
			"\\end{abstract}",
			"",
			"Conclusion paragraph.",
		].join("\n");

		const progress: number[] = [];
		await translateLatexBody(text, identityTranslator, {
			onProgress: (pct) => progress.push(pct),
		});

		for (let i = 1; i < progress.length; i++) {
			expect(progress[i]).toBeGreaterThanOrEqual(progress[i - 1]);
		}
		expect(progress.at(-1)).toBe(100);
	});

	it("reports monotonic progress through figure captions", async () => {
		const text = [
			"Before the figure.",
			"",
			"\\begin{figure}",
			"\\includegraphics{plot.png}",
			"\\caption{This is the caption text.}",
			"\\end{figure}",
			"",
			"After the figure.",
		].join("\n");

		const progress: number[] = [];
		await translateLatexBody(text, identityTranslator, {
			onProgress: (pct) => progress.push(pct),
		});

		for (let i = 1; i < progress.length; i++) {
			expect(progress[i]).toBeGreaterThanOrEqual(progress[i - 1]);
		}
		expect(progress.at(-1)).toBe(100);
	});
});
