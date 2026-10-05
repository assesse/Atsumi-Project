export const tutorialActionEvent = "atsumi:tutorial-action";
export const tutorialStepEvent = "atsumi:tutorial-step";
export const tutorialFinishedEvent = "atsumi:tutorial-finished";
export type TutorialActionResult = { stepId: string; visit: string; phase: "pending" | "success" | "error"; message?: string };

/** Reports the real operation result only to the visit that initiated it. No stored events. */
export function beginTutorialAction(stepId: string): (error?: string) => void {
  const state = document.documentElement.dataset;
  if (state.tutorialOpen !== "true" || state.tutorialStep !== stepId || !state.tutorialVisit) return () => {};
  const visit = state.tutorialVisit;
  const report = (phase: TutorialActionResult["phase"], message?: string) => {
    if (state.tutorialOpen !== "true" || state.tutorialStep !== stepId || state.tutorialVisit !== visit) return;
    window.dispatchEvent(new CustomEvent<TutorialActionResult>(tutorialActionEvent, { detail: { stepId, visit, phase, message } }));
  };
  report("pending");
  return error => report(error ? "error" : "success", error);
}
