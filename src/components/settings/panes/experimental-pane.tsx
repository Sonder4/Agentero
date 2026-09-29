import { Loader2 } from "lucide-react";
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { PageTitle } from "@/components/settings/settings-layout";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import { commands } from "@/lib/core/bindings";
import { errorText } from "@/lib/core/error";
import { callApiResult } from "@/lib/core/ipc";
import { notifyError } from "@/lib/core/notify";
import { cn } from "@/lib/core/utils";
import { saveSettingsAsync } from "@/lib/settings";
import { DEFAULT_JEV_BASE_URL } from "@/lib/settings/defaults";
import type { AppSettings, JevSettings } from "@/lib/settings/types";

type ProbeStatus = "idle" | "probing" | "ok" | "failed" | "unconfigured";

/** Same convention the Host uses for redacted secrets: all `*`, same length. */
function isMask(value: string): boolean {
	const trimmed = value.trim();
	return trimmed.length > 0 && trimmed.split("").every((c) => c === "*");
}

function maskKey(value: string): string {
	return value ? "*".repeat(value.length) : "";
}

function dotClass(status: ProbeStatus): string {
	switch (status) {
		case "ok":
			return "bg-emerald-500";
		case "failed":
			return "bg-destructive";
		case "probing":
			return "bg-amber-500 animate-pulse";
		case "unconfigured":
			return "bg-muted-foreground/35";
		default:
			return "bg-muted-foreground/50";
	}
}

export function ExperimentalPane({
	settings,
	patch,
}: {
	settings: AppSettings;
	patch: (p: Partial<AppSettings>) => void;
}) {
	const { t } = useTranslation("settings");
	const jev = settings.jev;
	const [probeStatus, setProbeStatus] = useState<ProbeStatus>(
		jev.apiKey.trim() ? "idle" : "unconfigured",
	);
	const [draft, setDraft] = useState<Partial<JevSettings>>({});
	const [busy, setBusy] = useState(false);

	const displayApiKey =
		draft.apiKey !== undefined
			? draft.apiKey
			: jev.apiKey
				? isMask(jev.apiKey)
					? jev.apiKey
					: maskKey(jev.apiKey)
				: "";

	const displayBaseUrl =
		draft.baseUrl !== undefined ? draft.baseUrl : jev.baseUrl;

	const handleConfirm = useCallback(async () => {
		const nextApiKey = (draft.apiKey ?? jev.apiKey).trim();
		const nextBaseUrl = (draft.baseUrl ?? jev.baseUrl).trim();

		const nextSettings: AppSettings = {
			...settings,
			jev: {
				apiKey: nextApiKey,
				baseUrl: nextBaseUrl,
			},
		};

		setBusy(true);
		try {
			const saved = await saveSettingsAsync(nextSettings);
			patch({ jev: saved.jev });
			setDraft({});

			if (!nextApiKey) {
				setProbeStatus("unconfigured");
				return;
			}

			setProbeStatus("probing");
			try {
				await callApiResult(() => commands.jevProbeHealth());
				setProbeStatus("ok");
			} catch (err) {
				setProbeStatus("failed");
				notifyError(errorText(err));
			}
		} catch (err) {
			notifyError(errorText(err));
		} finally {
			setBusy(false);
		}
	}, [draft, jev, patch, settings]);

	return (
		<div className="space-y-6">
			<PageTitle title={t("experimental.title")} />

			<div>
				<h3 className="mb-2 px-0.5 font-medium text-sm">
					{t("experimental.jev.section")}
				</h3>
				<div className="rounded-lg border bg-card px-3 py-2.5">
					<div className="mb-2 flex items-center justify-between gap-2">
						<div className="flex min-w-0 items-center gap-1.5">
							<Tooltip>
								<TooltipTrigger asChild>
									<span
										role="status"
										aria-label={t(
											`experimental.jev.probeStatus.${probeStatus}`,
										)}
										className={cn(
											"inline-block size-1.5 shrink-0 rounded-full",
											dotClass(probeStatus),
										)}
									/>
								</TooltipTrigger>
								<TooltipContent>
									{t(`experimental.jev.probeStatus.${probeStatus}`)}
								</TooltipContent>
							</Tooltip>
							<span className="truncate font-medium text-sm">
								{t("experimental.jev.section")}
							</span>
						</div>
						<Button
							type="button"
							variant="outline"
							size="xs"
							disabled={busy}
							onClick={() => void handleConfirm()}
						>
							{busy ? (
								<Loader2 className="mr-1.5 size-3.5 animate-spin" aria-hidden />
							) : null}
							{t("experimental.jev.test")}
						</Button>
					</div>

					<div className="grid gap-1.5">
						<div className="flex items-center gap-2">
							<Label
								htmlFor="jev-api-key"
								className="w-20 shrink-0 font-normal text-muted-foreground text-xs"
							>
								{t("experimental.jev.apiKey.label")}
							</Label>
							<Input
								id="jev-api-key"
								type="password"
								value={displayApiKey}
								placeholder={t("experimental.jev.apiKey.placeholder")}
								className="h-8 min-w-0 flex-1 font-mono text-xs placeholder:text-muted-foreground/50"
								spellCheck={false}
								autoComplete="off"
								onChange={(e) =>
									setDraft((prev) => ({ ...prev, apiKey: e.target.value }))
								}
								onFocus={(e) => e.target.select()}
							/>
						</div>
						<div className="flex items-center gap-2">
							<Label
								htmlFor="jev-base-url"
								className="w-20 shrink-0 font-normal text-muted-foreground text-xs"
							>
								{t("experimental.jev.baseUrl.label")}
							</Label>
							<Input
								id="jev-base-url"
								type="text"
								value={displayBaseUrl}
								placeholder={DEFAULT_JEV_BASE_URL}
								className="h-8 min-w-0 flex-1 font-mono text-xs placeholder:text-muted-foreground/50"
								spellCheck={false}
								autoComplete="off"
								onChange={(e) =>
									setDraft((prev) => ({ ...prev, baseUrl: e.target.value }))
								}
							/>
						</div>
					</div>
				</div>
			</div>
		</div>
	);
}
