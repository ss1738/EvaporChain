import { useState } from "react";

const TUTORIAL_KEY = "evaporchain_tutorial_complete";

interface TutorialStep {
  illustration: string;
  title: string;
  description: string;
}

const STEPS: TutorialStep[] = [
  {
    illustration: "~",
    title: "Objects Have Energy",
    description:
      "Objects on EvaporChain have energy that decays over time. Each object follows an exponential decay curve determined by its half-life.",
  },
  {
    illustration: "+",
    title: "Refresh or Evaporate",
    description:
      "Refresh objects to keep them alive by depositing energy. If you don't, their energy will eventually reach zero and they'll evaporate forever.",
  },
  {
    illustration: "!",
    title: "You're Ready!",
    description:
      "Claim free EVAP from the faucet to start experimenting. Create objects, watch them decay, and learn to manage energy efficiently.",
  },
];

export function OnboardingTutorial({ onComplete }: { onComplete: () => void }) {
  const [step, setStep] = useState(0);
  const [slideDirection, setSlideDirection] = useState<"left" | "right">("right");

  const current = STEPS[step];
  const isLast = step === STEPS.length - 1;

  const goNext = () => {
    if (isLast) {
      markComplete();
      onComplete();
      return;
    }
    setSlideDirection("right");
    setStep((s) => s + 1);
  };

  const skip = () => {
    markComplete();
    onComplete();
  };

  const markComplete = () => {
    try {
      localStorage.setItem(TUTORIAL_KEY, "true");
    } catch {
      // localStorage may not be available
    }
  };

  return (
    <div className="flex flex-col h-full bg-evap-bg">
      {/* Skip button */}
      <div className="flex justify-end px-4 pt-3">
        {!isLast && (
          <button
            onClick={skip}
            className="text-xs text-zinc-500 hover:text-zinc-300 transition"
          >
            Skip
          </button>
        )}
      </div>

      {/* Content area */}
      <div className="flex-1 flex flex-col items-center justify-center px-8">
        {/* Illustration */}
        <div
          className="w-24 h-24 rounded-2xl bg-evap-surface border border-evap-border flex items-center justify-center mb-8 transition-transform duration-300"
          style={{
            transform: `translateX(${slideDirection === "right" ? "0" : "0"}px)`,
          }}
        >
          <span className="text-4xl text-evap-cyan">{current.illustration}</span>
        </div>

        {/* Title */}
        <h2 className="text-lg font-bold text-zinc-100 text-center mb-3">
          {current.title}
        </h2>

        {/* Description */}
        <p className="text-sm text-zinc-400 text-center leading-relaxed max-w-[280px]">
          {current.description}
        </p>
      </div>

      {/* Bottom controls */}
      <div className="px-6 pb-6">
        {/* Step indicators */}
        <div className="flex items-center justify-center gap-2 mb-6">
          {STEPS.map((_, i) => (
            <div
              key={i}
              className={`rounded-full transition-all duration-300 ${
                i === step
                  ? "w-6 h-2 bg-evap-cyan"
                  : "w-2 h-2 bg-zinc-600"
              }`}
            />
          ))}
        </div>

        {/* Action button */}
        <button
          onClick={goNext}
          className="w-full py-3 rounded-xl bg-evap-cyan text-zinc-900 text-sm font-semibold hover:bg-evap-cyan/90 transition"
        >
          {isLast ? "Get Started" : "Next"}
        </button>
      </div>
    </div>
  );
}

/**
 * Check if the user has completed the onboarding tutorial.
 */
export function isTutorialComplete(): boolean {
  try {
    return localStorage.getItem(TUTORIAL_KEY) === "true";
  } catch {
    return false;
  }
}
