import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderSearch, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import type { AppConfig, AutoScanConfig, BackendError } from "@/types";
import { useErrorStore } from "./store";

const BYTES_PER_GB = 1024 * 1024 * 1024;

const formatThresholdGb = (bytes: number) => {
  const value = bytes / BYTES_PER_GB;
  return Number.isInteger(value) ? value.toString() : value.toFixed(1);
};

export function AutoScanSettings() {
  const { t } = useTranslation();
  const setCurrentBackendError = useErrorStore(
    (state) => state.setCurrentBackendError
  );
  const [config, setConfig] = useState<AutoScanConfig | null>(null);
  const [intervalDraft, setIntervalDraft] = useState("7");
  const [thresholdDraft, setThresholdDraft] = useState("1");
  const [isSaving, setIsSaving] = useState(false);

  useEffect(() => {
    const load = async () => {
      try {
        const result = await invoke<AppConfig>("get_auto_scan_config");
        setConfig(result.auto_scan);
        setIntervalDraft(result.auto_scan.interval_days.toString());
        setThresholdDraft(formatThresholdGb(result.auto_scan.save_threshold_bytes));
      } catch (err) {
        setCurrentBackendError(err as BackendError);
      }
    };

    load();
  }, [setCurrentBackendError]);

  const saveConfig = async (nextConfig: AutoScanConfig) => {
    const previousConfig = config;
    setConfig(nextConfig);
    setIsSaving(true);

    try {
      const result = await invoke<AppConfig>("update_auto_scan_config", {
        config: nextConfig,
      });
      setConfig(result.auto_scan);
      setIntervalDraft(result.auto_scan.interval_days.toString());
      setThresholdDraft(formatThresholdGb(result.auto_scan.save_threshold_bytes));
    } catch (err) {
      if (previousConfig) {
        setConfig(previousConfig);
        setIntervalDraft(previousConfig.interval_days.toString());
        setThresholdDraft(formatThresholdGb(previousConfig.save_threshold_bytes));
      }
      setCurrentBackendError(err as BackendError);
    } finally {
      setIsSaving(false);
    }
  };

  const chooseFolder = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
      });

      if (selected && typeof selected === "string" && config) {
        await saveConfig({ ...config, target_path: selected });
      }
    } catch (err) {
      setCurrentBackendError(err as BackendError);
    }
  };

  const handleEnabledChange = async (enabled: boolean) => {
    if (!config) return;

    if (enabled && config.target_path.trim() === "") {
      try {
        const selected = await open({
          directory: true,
          multiple: false,
        });

        if (selected && typeof selected === "string") {
          await saveConfig({ ...config, target_path: selected, enabled: true });
        }
      } catch (err) {
        setCurrentBackendError(err as BackendError);
      }
      return;
    }

    await saveConfig({ ...config, enabled });
  };

  const commitInterval = () => {
    if (!config) return;
    const intervalDays = Math.max(1, Math.floor(Number(intervalDraft) || 1));
    setIntervalDraft(intervalDays.toString());
    saveConfig({ ...config, interval_days: intervalDays });
  };

  const commitThreshold = () => {
    if (!config) return;
    const thresholdGb = Math.max(0, Number(thresholdDraft) || 0);
    const saveThresholdBytes = Math.round(thresholdGb * BYTES_PER_GB);
    setThresholdDraft(formatThresholdGb(saveThresholdBytes));
    saveConfig({ ...config, save_threshold_bytes: saveThresholdBytes });
  };

  if (!config) {
    return null;
  }

  const lastRunDate = config.last_scan_at
    ? new Date(config.last_scan_at).toLocaleString()
    : null;

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-base">{t("autoScan.title")}</CardTitle>
        <CardDescription>{t("autoScan.description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex items-center justify-between gap-4">
          <div className="space-y-1">
            <Label className="text-base font-medium">
              {t("autoScan.enableOnStartup")}
            </Label>
            <p className="text-[0.8rem] text-muted-foreground">
              {t("autoScan.enableDescription")}
            </p>
          </div>
          <Switch
            checked={config.enabled}
            disabled={isSaving}
            onCheckedChange={handleEnabledChange}
          />
        </div>

        <div className="space-y-2">
          <Label>{t("autoScan.targetPath")}</Label>
          <div className="flex gap-2">
            <Input
              value={config.target_path}
              readOnly
              placeholder={t("autoScan.targetPlaceholder")}
            />
            <Button
              type="button"
              variant="outline"
              size="icon"
              onClick={chooseFolder}
              disabled={isSaving}
              aria-label={t("autoScan.chooseFolder")}
            >
              <FolderSearch className="h-4 w-4" />
            </Button>
          </div>
          {config.target_path.trim() === "" && (
            <p className="text-[0.8rem] text-muted-foreground">
              {t("autoScan.targetRequired")}
            </p>
          )}
        </div>

        <div className="grid grid-cols-2 gap-3">
          <div className="space-y-2">
            <Label>{t("autoScan.intervalDays")}</Label>
            <Input
              type="number"
              min={1}
              value={intervalDraft}
              disabled={isSaving}
              onChange={(event) => setIntervalDraft(event.target.value)}
              onBlur={commitInterval}
            />
          </div>
          <div className="space-y-2">
            <Label>{t("autoScan.thresholdGb")}</Label>
            <Input
              type="number"
              min={0}
              step={0.1}
              value={thresholdDraft}
              disabled={isSaving}
              onChange={(event) => setThresholdDraft(event.target.value)}
              onBlur={commitThreshold}
            />
          </div>
        </div>

        <div className="rounded-md border bg-background p-3 text-sm">
          <div className="font-medium">{t("autoScan.lastRun")}</div>
          <div className="text-muted-foreground">
            {t(`autoScan.status.${config.last_status}`)}
          </div>
          {lastRunDate && (
            <div className="text-muted-foreground">{lastRunDate}</div>
          )}
          {config.last_error && (
            <div className="mt-1 text-red-600">{config.last_error}</div>
          )}
          {isSaving && (
            <div className="mt-2 flex items-center gap-2 text-muted-foreground">
              <Loader2 className="h-3 w-3 animate-spin" />
              {t("autoScan.saving")}
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
