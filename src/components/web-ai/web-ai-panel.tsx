import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
	commands,
	type WebAiProvider,
	type WebAiStatus,
} from "@/lib/core/bindings";
import { callApiResult } from "@/lib/core/ipc";
import { isMobileApp, isTauri } from "@/lib/core/tauri";
import { cn } from "@/lib/core/utils";

export function WebAiPanel() {
	const { t } = useTranslation("app");
	const mobileOnly = isMobileApp();
	const hostRef = useRef<HTMLDivElement>(null);
	const [providers, setProviders] = useState<WebAiProvider[]>([]);
	const [providerId, setProviderId] = useState("chatgpt");
	const [status, setStatus] = useState<WebAiStatus | null>(null);
	const [text, setText] = useState("");
	const [error, setError] = useState<string | null>(null);

	const refresh = useCallback(async () => {
		if (!isTauri() || mobileOnly) return;
		try {
			const [items, states] = await Promise.all([
				callApiResult(() => commands.webAiProviders()),
				callApiResult(() => commands.webAiStatus(null)),
			]);
			setProviders(items);
			setStatus(
				states.find((item) => item.activeProviderId === providerId) ?? null,
			);
		} catch (cause) {
			setError(cause instanceof Error ? cause.message : String(cause));
		}
	}, [mobileOnly, providerId]);

	useEffect(() => {
		void refresh();
	}, [refresh]);

	useEffect(() => {
		if (!isTauri() || mobileOnly || !hostRef.current) return;
		const host = hostRef.current;
		let frame = 0;
		const publish = () => {
			frame = 0;
			const rect = host.getBoundingClientRect();
			const scale = window.devicePixelRatio || 1;
			void callApiResult(() =>
				commands.webAiSetBounds({
					providerId,
					bounds: {
						x: rect.left,
						y: rect.top,
						width: rect.width,
						height: rect.height,
						scaleFactor: scale,
					},
				}),
			).catch(() => undefined);
		};
		const observer = new ResizeObserver(() => {
			if (!frame) frame = requestAnimationFrame(publish);
		});
		observer.observe(host);
		publish();
		return () => {
			observer.disconnect();
			if (frame) cancelAnimationFrame(frame);
			void callApiResult(() =>
				commands.webAiView({ providerId, visible: false }),
			).catch(() => undefined);
		};
	}, [mobileOnly, providerId]);

	const open = async () => {
		setError(null);
		try {
			const next = await callApiResult(() =>
				commands.webAiOpen({ providerId, bounds: null }),
			);
			setStatus(next);
		} catch (cause) {
			setError(cause instanceof Error ? cause.message : String(cause));
		}
	};

	const prepareText = async () => {
		if (!text.trim()) return;
		setError(null);
		try {
			await callApiResult(() =>
				commands.webAiTransferText({
					providerId,
					text,
					paperId: null,
					page: null,
				}),
			);
			setText("");
		} catch (cause) {
			setError(cause instanceof Error ? cause.message : String(cause));
		}
	};

	if (mobileOnly) {
		return (
			<div className="p-3 text-xs text-muted-foreground">
				{t("webAi.desktopOnly")}
			</div>
		);
	}

	return (
		<div className="flex h-full min-h-0 flex-col">
			<div className="flex shrink-0 items-center gap-2 border-b px-2 py-2">
				<select
					className="min-w-0 flex-1 rounded border bg-background px-2 py-1 text-xs"
					value={providerId}
					onChange={(event) => setProviderId(event.target.value)}
					aria-label={t("webAi.provider")}
				>
					{providers.map((provider) => (
						<option key={provider.id} value={provider.id}>
							{provider.name}
						</option>
					))}
				</select>
				<button
					type="button"
					className="rounded bg-primary px-2 py-1 text-xs text-primary-foreground"
					onClick={() => void open()}
				>
					{t("webAi.open")}
				</button>
			</div>
			<div className="flex shrink-0 items-center justify-between px-2 py-1 text-xs text-muted-foreground">
				<span>{status?.view ?? "closed"}</span>
				<span>{status?.authenticated ?? "unknown"}</span>
			</div>
			<div ref={hostRef} className="min-h-0 flex-1 bg-muted/20" />
			<div className="flex shrink-0 gap-2 border-t p-2">
				<textarea
					className="min-h-16 min-w-0 flex-1 resize-none rounded border bg-background px-2 py-1 text-xs"
					value={text}
					onChange={(event) => setText(event.target.value)}
					placeholder={t("webAi.textPlaceholder")}
				/>
				<button
					type="button"
					className={cn(
						"self-end rounded border px-2 py-1 text-xs",
						!text.trim() && "opacity-50",
					)}
					disabled={!text.trim()}
					onClick={() => void prepareText()}
				>
					{t("webAi.prepare")}
				</button>
			</div>
			{error && (
				<div className="shrink-0 px-2 pb-2 text-xs text-destructive">
					{error}
				</div>
			)}
		</div>
	);
}
