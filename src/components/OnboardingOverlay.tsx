/* eslint-disable react-refresh/only-export-components -- persistence helpers intentionally live with the overlay */
import { useState } from "react";
import {
  ArrowRight,
  Check,
  Cloud,
  FolderOpen,
  Music2,
  Radio,
  SkipForward,
  Sparkles,
} from "lucide-react";
import type { NetEaseServiceStatus } from "../features/musicSources/provider";

const ONBOARDING_STORAGE_KEY = "ome.onboarding.completed";

export function isOnboardingCompleted(): boolean {
  return window.localStorage.getItem(ONBOARDING_STORAGE_KEY) === "1";
}

export function markOnboardingCompleted(): void {
  window.localStorage.setItem(ONBOARDING_STORAGE_KEY, "1");
}

export function resetOnboarding(): void {
  window.localStorage.removeItem(ONBOARDING_STORAGE_KEY);
}

interface OnboardingOverlayProps {
  serviceStatus: NetEaseServiceStatus | null;
  localTrackCount: number;
  neteaseLoggedIn: boolean;
  bilibiliLoggedIn: boolean;
  qqmusicLoggedIn: boolean;
  onClose: () => void;
  onImportMusic: () => void;
  onOpenNeteaseSettings: () => void;
  onOpenBilibiliSettings: () => void;
  onOpenQQMusicSettings: () => void;
}

interface OnboardingStep {
  id: string;
  icon: typeof Music2;
  title: string;
  subtitle: string;
  body: string;
  cta?: { label: string; onClick: () => void };
  done?: boolean;
  doneLabel?: string;
}

export function OnboardingOverlay({
  serviceStatus,
  localTrackCount,
  neteaseLoggedIn,
  bilibiliLoggedIn,
  qqmusicLoggedIn,
  onClose,
  onImportMusic,
  onOpenNeteaseSettings,
  onOpenBilibiliSettings,
  onOpenQQMusicSettings,
}: OnboardingOverlayProps) {
  const [stepIndex, setStepIndex] = useState(0);

  const handleFinish = () => {
    markOnboardingCompleted();
    onClose();
  };

  const handleSkip = () => {
    markOnboardingCompleted();
    onClose();
  };

  const musicImported = localTrackCount > 0;
  const neteaseReady = Boolean(serviceStatus?.running);

  const steps: OnboardingStep[] = [
    {
      id: "welcome",
      icon: Sparkles,
      title: "\u6b22\u8fce\u6765\u5230 Ome Music",
      subtitle: "Welcome",
      body: "A quiet, immersive desktop music player. Start with local music, then connect NetEase Cloud Music or Bilibili when you are ready.",
    },
    {
      id: "local",
      icon: FolderOpen,
      title: "\u5bfc\u5165\u672c\u5730\u97f3\u4e50",
      subtitle: "Local Music",
      body: "Choose a music folder on your computer. Ome Music only saves file paths and never copies your original music files.",
      cta: musicImported
        ? undefined
        : { label: "\u5bfc\u5165\u97f3\u4e50", onClick: onImportMusic },
      done: musicImported,
      doneLabel: `\u5df2\u5bfc\u5165 ${localTrackCount} \u9996`,
    },
    {
      id: "netease",
      icon: Cloud,
      title: "\u8fde\u63a5\u7f51\u6613\u4e91\u97f3\u4e50",
      subtitle: "NetEase Cloud Music",
      body: neteaseReady
        ? "The NetEase music source is ready. Open Settings, scan the QR code, and play tracks available to your own account."
        : "The Windows installer includes the NetEase music source. If it is unavailable, reinstall the latest Ome Music build.",
      cta: { label: "\u6253\u5f00\u8bbe\u7f6e", onClick: onOpenNeteaseSettings },
      done: neteaseLoggedIn,
      doneLabel: "\u5df2\u767b\u5f55",
    },
    {
      id: "bilibili",
      icon: Radio,
      title: "\u8fde\u63a5 Bilibili",
      subtitle: "Bilibili",
      body: "Bilibili can provide music, covers, video atmosphere, and a gentle danmaku layer after sign-in.",
      cta: { label: "\u6253\u5f00\u8bbe\u7f6e", onClick: onOpenBilibiliSettings },
      done: bilibiliLoggedIn,
      doneLabel: "\u5df2\u767b\u5f55",
    },
    {
      id: "qqmusic",
      icon: Cloud,
      title: "\u8fde\u63a5 QQ\u97f3\u4e50",
      subtitle: "QQ Music",
      body: "QQ Music adds another rich catalog. Sign in to unlock high-quality streaming and your playlists.",
      cta: { label: "\u6253\u5f00\u8bbe\u7f6e", onClick: onOpenQQMusicSettings },
      done: qqmusicLoggedIn,
      doneLabel: "\u5df2\u767b\u5f55",
    },
    {
      id: "ready",
      icon: Check,
      title: "\u53ef\u4ee5\u5f00\u59cb\u542c\u4e86",
      subtitle: "All Set",
      body: "Search from the top bar and click a result to play. Settings manages sources, lyrics, danmaku, and storage.",
    },
  ];

  const currentStep = steps[stepIndex];
  const isLastStep = stepIndex === steps.length - 1;
  const StepIcon = currentStep.icon;
  const progress = ((stepIndex + 1) / steps.length) * 100;

  const handleNext = () => {
    if (isLastStep) {
      handleFinish();
    } else {
      setStepIndex((index) => Math.min(index + 1, steps.length - 1));
    }
  };

  const handlePrev = () => {
    setStepIndex((index) => Math.max(index - 1, 0));
  };

  return (
    <div className="fixed inset-0 z-[80] flex items-center justify-center bg-[#4a2108]/[0.18] backdrop-blur-sm">
      <div
        data-danmaku-safe-zone="onboarding"
        className="relative w-[min(560px,calc(100vw-3rem))] rounded-[32px] border border-white/40 bg-[#f5ece1]/95 p-8 text-[#4a2108] shadow-[0_32px_96px_rgba(74,33,8,0.28)]"
      >
        <button
          type="button"
          onClick={handleSkip}
          className="app-transition absolute right-5 top-5 flex items-center gap-1.5 rounded-full bg-[#4a2108]/[0.05] px-3 py-1.5 text-[11px] font-bold text-[#4a2108]/48 hover:bg-[#4a2108]/[0.1] hover:text-[#4a2108]/72"
        >
          <SkipForward className="h-3.5 w-3.5" />
          {"\u8df3\u8fc7"}
        </button>

        <div className="mb-6 mt-2 h-1 w-full overflow-hidden rounded-full bg-[#4a2108]/[0.06]">
          <div
            className="app-transition h-full rounded-full bg-gradient-to-r from-[#c66043]/70 to-[#d4a05a]/80"
            style={{ width: `${progress}%` }}
          />
        </div>

        <div className="mb-6 flex items-center justify-center gap-2">
          {steps.map((step, index) => (
            <button
              key={step.id}
              type="button"
              onClick={() => setStepIndex(index)}
              className={`h-1.5 rounded-full app-transition ${
                index === stepIndex
                  ? "w-8 bg-[#c66043]/70"
                  : index < stepIndex
                    ? "w-1.5 bg-[#638052]/55"
                    : "w-1.5 bg-[#4a2108]/12"
              }`}
              aria-label={`Step ${index + 1}: ${step.title}`}
            />
          ))}
        </div>

        <div className="mb-5 flex justify-center">
          <div className="flex h-20 w-20 items-center justify-center rounded-[24px] bg-gradient-to-br from-[#c66043]/12 to-[#d4a05a]/10 shadow-[0_8px_24px_rgba(198,96,67,0.12)]">
            <StepIcon className="h-9 w-9 text-[#7a2d1c]/65" strokeWidth={1.6} />
          </div>
        </div>

        <div className="mb-3 text-center">
          <p className="text-[10px] font-black uppercase tracking-[0.22em] text-[#4a2108]/32">
            {currentStep.subtitle}
          </p>
          <h2 className="mt-1.5 text-[22px] font-bold text-[#4a2108]/88">{currentStep.title}</h2>
        </div>

        <p className="mx-auto mb-6 max-w-[420px] text-center text-[13.5px] font-medium leading-[1.7] text-[#4a2108]/64">
          {currentStep.body}
        </p>

        {currentStep.done && currentStep.doneLabel && (
          <div className="mb-5 flex justify-center">
            <span className="flex items-center gap-1.5 rounded-full bg-[#638052]/12 px-3 py-1 text-[11px] font-bold text-[#3d5230]/72">
              <Check className="h-3.5 w-3.5" />
              {currentStep.doneLabel}
            </span>
          </div>
        )}

        {currentStep.cta && (
          <div className="mb-5 flex justify-center">
            <button
              type="button"
              onClick={currentStep.cta.onClick}
              className="app-transition flex items-center gap-2 rounded-full bg-[#4a2108]/[0.08] px-5 py-2.5 text-xs font-black text-[#4a2108]/72 hover:bg-[#4a2108]/[0.14] hover:text-[#4a2108]/92"
            >
              {currentStep.cta.label}
              <ArrowRight className="h-3.5 w-3.5" />
            </button>
          </div>
        )}

        <div className="flex items-center justify-between border-t border-[#4a2108]/[0.06] pt-5">
          <button
            type="button"
            onClick={handlePrev}
            disabled={stepIndex === 0}
            className="app-transition rounded-full px-4 py-2 text-xs font-bold text-[#4a2108]/38 hover:text-[#4a2108]/62 disabled:opacity-30"
          >
            {"\u4e0a\u4e00\u6b65"}
          </button>

          <span className="text-[10px] font-semibold text-[#4a2108]/28">
            {stepIndex + 1} / {steps.length}
          </span>

          <button
            type="button"
            onClick={handleNext}
            className="app-transition flex items-center gap-1.5 rounded-full bg-[#c66043]/[0.14] px-5 py-2 text-xs font-black text-[#7a2d1c]/82 hover:bg-[#c66043]/[0.22] hover:text-[#7a2d1c]"
          >
            {isLastStep ? "\u5f00\u59cb\u4f7f\u7528" : "\u4e0b\u4e00\u6b65"}
            {!isLastStep && <ArrowRight className="h-3.5 w-3.5" />}
            {isLastStep && <Music2 className="h-3.5 w-3.5" />}
          </button>
        </div>
      </div>
    </div>
  );
}
