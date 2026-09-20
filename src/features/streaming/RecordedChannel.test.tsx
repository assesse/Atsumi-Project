import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, it, expect, vi } from "vitest";
import { RecordedChannel } from "./RecordedChannel";
import { openRecordingChannel, recordingProfile } from "../../api/recordingProfile";
vi.mock("../../api/recordingProfile", () => ({ recordingProfile: vi.fn(), openRecordingChannel: vi.fn() }));
let root: Root, container: HTMLDivElement;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.clearAllMocks();
  vi.mocked(recordingProfile).mockResolvedValue({ name: "당시 채널", image: "data:image/png;base64,YQ==" });
  vi.mocked(openRecordingChannel).mockResolvedValue();
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });
it("renders the archived identity and opens only the stored channel on an explicit click", async () => {
  await act(async () => root.render(<RecordedChannel id="recording-1" />));
  expect(container).toHaveTextContent("당시 채널");
  expect(container.querySelector("img")).toHaveAttribute("src", "data:image/png;base64,YQ==");
  expect(openRecordingChannel).not.toHaveBeenCalled();
  await act(async () => container.querySelector("button")!.click());
  expect(openRecordingChannel).toHaveBeenCalledExactlyOnceWith("recording-1");
});
it("never loads remote profile images and respects privacy", async () => {
  await act(async () => root.render(<RecordedChannel id="recording-1" name="name" image="https://example.invalid/private" />));
  expect(container.querySelector("img")).toBeNull();
  expect(container).toHaveTextContent("당시 프로필 이미지 없음");
  await act(async () => root.render(<RecordedChannel id="recording-2" privacy />));
  expect(container).toBeEmptyDOMElement(); expect(recordingProfile).not.toHaveBeenCalled();
});
