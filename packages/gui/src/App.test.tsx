// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  captionTextForTime,
  changesFromEvents,
  fullscreenAction,
  setViewerAudioVolume,
  subtitleInfoForSource,
  toggleOverlay,
  toggleViewerAudio,
  updateViewerOptions,
  type ViewerOptions,
  type ViewerAudioState,
} from "./App";
import App from "./App";
import {
  acceptProposal,
  getAgentConfig,
  getRender,
  loadState,
  rejectProposal,
  sendCommand,
  startRender,
  updateAgentConfig,
} from "./api";
import type { Project, Source, StatePayload } from "./api";

vi.mock("./api", () => ({
  ApiError: class ApiError extends Error {
    status = 0;
    code?: string;
    get isConflict() { return false; }
  },
  acceptProposal: vi.fn(),
  getAgentConfig: vi.fn(),
  getRender: vi.fn(),
  importMedia: vi.fn(),
  isDesktopRuntime: vi.fn(() => false),
  loadState: vi.fn(),
  mediaUrl: vi.fn((sourceId: string) => `/api/media/${sourceId}`),
  outputUrl: vi.fn((jobId: string) => `/api/output/${jobId}`),
  pickMediaFile: vi.fn(),
  redo: vi.fn(),
  rejectProposal: vi.fn(),
  resolveAssetUrl: vi.fn(),
  resetAgentSession: vi.fn(),
  sendCommand: vi.fn(),
  startRender: vi.fn(),
  stateFromResponse: vi.fn((value: unknown) => value),
  undo: vi.fn(),
  updateAgentConfig: vi.fn(),
}));

const testState: StatePayload = {
  project: {
    id: "demo-project",
    name: "Demo project",
    version: 2,
    fps: 30,
    sources: [{ id: "source-1", path: "/tmp/demo.mp4" }],
    tracks: [{ id: "video-1", type: "video", clips: [{ id: "clip-1", src: "source-1", in: 0, out: 4, volume: 1 }] }],
    overlays: [{ type: "captions", cues: [{ start: 0, end: 3, text: "Test caption" }] }],
    gaps: [],
    transitions: [],
    effects: [],
    markers: [],
  } as Project,
  revision: 1,
  canUndo: false,
  canRedo: false,
  duration: 4,
  sources: [{ id: "source-1", path: "/tmp/demo.mp4", title: "Demo source", duration: 4 }],
};

const testAgentConfig = {
  provider: "openai",
  model: "gpt-5.4",
  models: [
    { id: "gpt-5.4", label: "GPT-5.4", provider: "openai" },
    { id: "gpt-5.6", label: "GPT-5.6", provider: "openai" },
  ],
};

const mockLoadState = vi.mocked(loadState);
const mockGetAgentConfig = vi.mocked(getAgentConfig);
const mockGetRender = vi.mocked(getRender);
const mockAcceptProposal = vi.mocked(acceptProposal);
const mockRejectProposal = vi.mocked(rejectProposal);
const mockSendCommand = vi.mocked(sendCommand);
const mockStartRender = vi.mocked(startRender);
const mockUpdateAgentConfig = vi.mocked(updateAgentConfig);

beforeEach(() => {
  mockLoadState.mockResolvedValue(testState);
  mockGetAgentConfig.mockResolvedValue(testAgentConfig);
  mockGetRender.mockResolvedValue({ jobId: "render-1", status: "completed", outputUrl: "/out/demo.mp4" });
  mockSendCommand.mockResolvedValue(testState);
  mockStartRender.mockResolvedValue({ jobId: "render-1", status: "queued" });
  mockUpdateAgentConfig.mockResolvedValue(testAgentConfig);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("GUI agent activity", () => {
  it("filters stream lifecycle events and keeps meaningful tool changes compact", () => {
    const changes = changesFromEvents([
      { type: "text_delta", text: "Hello" },
      { type: "text_delta", text: " world" },
      { type: "turn_started" },
      { type: "tool_call_started", id: "call-1", name: "project_edit" },
      { type: "tool_call_finished", id: "call-1", name: "project_edit", ok: true, result: { revision: 1 } },
      { type: "tool_call_finished", id: "read-1", name: "timeline_query", ok: true, result: { tracks: [] } },
      { type: "tool_call_finished", id: "call-2", name: "project_edit", ok: true, result: { revision: 1 } },
      { type: "done", text: "Finished" },
      { type: "result", text: "Finished" },
    ]);

    expect(changes).toHaveLength(1);
    expect(changes[0]).toMatchObject({ title: "Project_edit", range: "Project update" });
  });

  it("treats unified read-only agent tools as non-changes when successful", () => {
    const changes = changesFromEvents([
      { type: "tool_call_finished", id: "probe-1", name: "media.probe", ok: true, result: { duration: 12 } },
      { type: "tool_call_finished", id: "fetch-1", name: "media.fetch", ok: true, result: { sourceId: "src-1" } },
      { type: "tool_call_finished", id: "import-1", name: "media.import", ok: true, result: { sourceId: "src-2" } },
      { type: "tool_call_finished", id: "validate-1", name: "project.validate", ok: true, result: { valid: true } },
      { type: "tool_call_finished", id: "diff-1", name: "timeline.diff", ok: true, result: { changes: [] } },
      { type: "tool_call_finished", id: "track-1", name: "vision.track_objects", ok: true, result: { tracks: [] } },
      { type: "tool_call_finished", id: "status-1", name: "render.status", ok: true, result: { status: "completed" } },
      { type: "tool_call_finished", id: "result-1", name: "render.result", ok: true, result: { outputUrl: "/out.mp4" } },
      { type: "tool_call_finished", id: "apply-1", name: "timeline_apply", ok: true, result: { revision: 2 } },
    ]);

    expect(changes.map((change) => change.title)).toEqual([
      "Media.fetch",
      "Media.import",
      "Timeline_apply",
    ]);
  });

  it("detects imported subtitle metadata and caption overlays", () => {
    const source: Source = {
      id: "youtube",
      path: "/media/video.mp4",
      subs: { language: "en", path: "/media/video.en.vtt" },
    };
    expect(subtitleInfoForSource(source)).toMatchObject({ available: true, label: "en", path: "/media/video.en.vtt" });

    const project: Project = {
      version: 2,
      fps: 30,
      sources: [],
      tracks: [],
      overlays: [{ type: "captions", cues: [{ start: 1, end: 3, text: "Hello from YouTube" }] }],
      gaps: [],
      transitions: [],
      effects: [],
      markers: [],
    };
    expect(captionTextForTime(project, 2)).toBe("Hello from YouTube");
    expect(captionTextForTime(project, 3)).toBeUndefined();
  });
});

describe("GUI local controls", () => {
  it("opens one typed overlay at a time and toggles it closed", () => {
    expect(toggleOverlay(null, "project-menu")).toBe("project-menu");
    expect(toggleOverlay("project-menu", "project-menu")).toBeNull();
    expect(toggleOverlay("settings", "viewer-options")).toBe("viewer-options");
  });

  it("keeps viewer audio in the local 0..1 range and restores the last level", () => {
    const initial: ViewerAudioState = { volume: 0.7, muted: false, lastVolume: 0.7 };
    const muted = toggleViewerAudio(initial);
    expect(muted).toEqual({ volume: 0.7, muted: true, lastVolume: 0.7 });
    expect(toggleViewerAudio(muted)).toEqual(initial);
    expect(setViewerAudioVolume(initial, 1.5)).toEqual({ volume: 1, muted: false, lastVolume: 1 });
    expect(setViewerAudioVolume(initial, -1)).toEqual({ volume: 0, muted: true, lastVolume: 0.7 });
  });

  it("switches the viewer between before/after and local captions", () => {
    const initial: ViewerOptions = { showAfter: true, captionsVisible: true };
    expect(updateViewerOptions(initial, { showAfter: false })).toEqual({ showAfter: false, captionsVisible: true });
    expect(updateViewerOptions(initial, { captionsVisible: false })).toEqual({ showAfter: true, captionsVisible: false });
  });

  it("chooses fullscreen enter/exit from the standard document state", () => {
    expect(fullscreenAction(false, false)).toBe("enter");
    expect(fullscreenAction(true, true)).toBe("exit");
    expect(fullscreenAction(false, true)).toBe("exit");
  });
});

describe("GUI rendered controls", () => {
  async function renderApp() {
    render(<App />);
    await waitFor(() => expect(screen.getAllByText("Demo project").length).toBeGreaterThan(0));
  }

  it("opens the project menu and performs supported project actions", async () => {
    await renderApp();
    fireEvent.click(screen.getByRole("button", { name: "Project menu" }));
    const menu = screen.getByRole("menu", { name: "Project actions" });
    expect(within(menu).getByRole("menuitem", { name: /Manage media/ })).toBeTruthy();
    expect(within(menu).getByRole("menuitem", { name: /Refresh state/ })).toBeTruthy();
    expect(within(menu).getByRole("menuitem", { name: /Preview/ })).toBeTruthy();
    expect(within(menu).getByRole("menuitem", { name: /Export/ })).toBeTruthy();
    expect(within(menu).queryByText(/Open|New|Save/)).toBeNull();

    const loadCallsBeforeRefresh = mockLoadState.mock.calls.length;
    fireEvent.click(within(menu).getByRole("menuitem", { name: /Refresh state/ }));
    await waitFor(() => expect(mockLoadState.mock.calls.length).toBeGreaterThan(loadCallsBeforeRefresh));

    fireEvent.click(screen.getByRole("button", { name: "Project menu" }));
    const refreshedMenu = screen.getByRole("menu", { name: "Project actions" });
    fireEvent.click(within(refreshedMenu).getByRole("menuitem", { name: /Manage media/ }));
    expect(screen.getByRole("dialog", { name: "Manage media" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Close media manager" }));
    fireEvent.click(screen.getByRole("button", { name: "Project menu" }));
    fireEvent.click(screen.getByRole("menuitem", { name: /Preview/ }));
    await waitFor(() => expect(mockStartRender).toHaveBeenCalledWith("preview"));

    fireEvent.click(screen.getByRole("button", { name: "Project menu" }));
    fireEvent.click(screen.getByRole("menuitem", { name: /Export/ }));
    await waitFor(() => expect(mockStartRender).toHaveBeenCalledWith("full"));
  });

  it("shows a structured proposal and applies it only after acceptance", async () => {
    const proposal = {
      status: "pending" as const,
      proposalId: "proposal-1",
      baseRevision: testState.revision,
      diff: [{ path: "$.tracks[0].clips[0].out", kind: "changed", before: 4, after: 3 }],
    };
    mockLoadState.mockResolvedValue({ ...testState, pendingProposal: proposal });
    mockAcceptProposal.mockResolvedValue(testState);
    await renderApp();

    expect(screen.getByRole("region", { name: "Pending agent proposal" })).toBeTruthy();
    expect(screen.getByText("Update out")).toBeTruthy();
    expect(screen.getByText("4 → 3")).toBeTruthy();
    expect((screen.getByRole("textbox", { name: "Message the Cut agent" }) as HTMLTextAreaElement).disabled).toBe(true);
    expect(mockSendCommand).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Accept proposal" }));
    await waitFor(() => expect(mockAcceptProposal).toHaveBeenCalledWith("proposal-1", testState.revision));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Accept proposal" })).toBeNull());
    expect(mockSendCommand).not.toHaveBeenCalled();
  });

  it("rejects a structured proposal without sending a project command", async () => {
    const proposal = {
      status: "pending" as const,
      proposalId: "proposal-2",
      baseRevision: testState.revision,
      diff: [{ path: "$.metadata.intent", kind: "added", before: null, after: "review" }],
    };
    mockLoadState.mockResolvedValue({ ...testState, pendingProposal: proposal });
    mockRejectProposal.mockResolvedValue(testState);
    await renderApp();

    fireEvent.click(screen.getByRole("button", { name: "Reject proposal" }));
    await waitFor(() => expect(mockRejectProposal).toHaveBeenCalledWith("proposal-2", testState.revision));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Reject proposal" })).toBeNull());
    expect(mockSendCommand).not.toHaveBeenCalled();
  });

  it("shows agent settings and updates the configured model through the host", async () => {
    await renderApp();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    const settings = screen.getByRole("dialog", { name: "Settings" });
    expect(within(settings).getByText("openai")).toBeTruthy();
    const model = within(settings).getByRole("combobox", { name: "Model" }) as HTMLSelectElement;
    expect(model.value).toBe("gpt-5.4");
    fireEvent.change(model, { target: { value: "gpt-5.6" } });
    await waitFor(() => expect(mockUpdateAgentConfig).toHaveBeenCalledWith({ model: "gpt-5.6", provider: "openai" }));
  });

  it("changes before/after and caption visibility from viewer options", async () => {
    await renderApp();
    expect(document.querySelector(".caption-overlay")?.textContent).toBe("Test caption");
    fireEvent.click(screen.getByRole("button", { name: "Viewer options" }));
    const options = screen.getByRole("dialog", { name: "Viewer options" });
    fireEvent.click(within(options).getByRole("button", { name: "Before" }));
    expect(within(options).getByRole("button", { name: "Before" }).getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(within(options).getByRole("checkbox", { name: "Show captions" }));
    expect(document.querySelector(".caption-overlay")).toBeNull();
  });

  it("syncs viewer audio controls to the video without sending a project command", async () => {
    await renderApp();
    const video = document.querySelector("video") as HTMLVideoElement;
    fireEvent.click(screen.getByRole("button", { name: "Audio controls" }));
    const controls = screen.getByRole("dialog", { name: "Audio controls" });
    fireEvent.change(within(controls).getByRole("slider", { name: "Playback volume" }), { target: { value: "0.4" } });
    await waitFor(() => expect(video.volume).toBe(0.4));
    expect(video.muted).toBe(false);
    fireEvent.click(within(controls).getByRole("button", { name: "Mute" }));
    await waitFor(() => expect(video.muted).toBe(true));
    expect(mockSendCommand).not.toHaveBeenCalled();
    fireEvent.click(within(controls).getByRole("button", { name: "Unmute" }));
    await waitFor(() => expect(video.muted).toBe(false));
    expect(video.volume).toBe(0.4);
  });

  it("uses fullscreenchange as the source of truth and clears pressed state when enter resolves without fullscreen", async () => {
    await renderApp();
    const stage = document.querySelector(".viewer-stage") as HTMLDivElement;
    const previousRequestFullscreen = Object.getOwnPropertyDescriptor(HTMLDivElement.prototype, "requestFullscreen");
    const previousExitFullscreen = Object.getOwnPropertyDescriptor(document, "exitFullscreen");
    const previousFullscreenElement = Object.getOwnPropertyDescriptor(document, "fullscreenElement");
    const requestFullscreen = vi.fn().mockResolvedValue(undefined);
    const exitFullscreen = vi.fn().mockImplementation(async () => {
      Object.defineProperty(document, "fullscreenElement", { configurable: true, value: null });
      document.dispatchEvent(new Event("fullscreenchange"));
    });
    Object.defineProperty(HTMLDivElement.prototype, "requestFullscreen", { configurable: true, value: requestFullscreen });
    Object.defineProperty(document, "exitFullscreen", { configurable: true, value: exitFullscreen });
    Object.defineProperty(document, "fullscreenElement", { configurable: true, value: null });

    const fullscreen = screen.getByRole("button", { name: "Fullscreen" });
    fireEvent.click(fullscreen);
    await waitFor(() => expect(requestFullscreen).toHaveBeenCalledTimes(1));
    expect(fullscreen.getAttribute("aria-pressed")).toBe("false");

    Object.defineProperty(document, "fullscreenElement", { configurable: true, value: stage });
    document.dispatchEvent(new Event("fullscreenchange"));
    await waitFor(() => expect(screen.getByRole("button", { name: "Exit fullscreen" }).getAttribute("aria-pressed")).toBe("true"));

    Object.defineProperty(document, "fullscreenElement", { configurable: true, value: null });
    await new Promise((resolve) => window.setTimeout(resolve, 800));
    expect(screen.getByRole("button", { name: "Fullscreen" }).getAttribute("aria-pressed")).toBe("false");

    Object.defineProperty(document, "fullscreenElement", { configurable: true, value: stage });
    document.dispatchEvent(new Event("fullscreenchange"));
    await waitFor(() => expect(screen.getByRole("button", { name: "Exit fullscreen" }).getAttribute("aria-pressed")).toBe("true"));

    fireEvent.click(screen.getByRole("button", { name: "Exit fullscreen" }));
    await waitFor(() => expect(exitFullscreen).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.getByRole("button", { name: "Fullscreen" }).getAttribute("aria-pressed")).toBe("false"));

    if (previousRequestFullscreen) Object.defineProperty(HTMLDivElement.prototype, "requestFullscreen", previousRequestFullscreen);
    else Reflect.deleteProperty(HTMLDivElement.prototype, "requestFullscreen");
    if (previousExitFullscreen) Object.defineProperty(document, "exitFullscreen", previousExitFullscreen);
    else Reflect.deleteProperty(document, "exitFullscreen");
    if (previousFullscreenElement) Object.defineProperty(document, "fullscreenElement", previousFullscreenElement);
    else Reflect.deleteProperty(document, "fullscreenElement");
  });

  it("closes Adjust back into the single Chat rail", async () => {
    await renderApp();
    fireEvent.click(screen.getByRole("tab", { name: /Adjust/ }));
    expect(screen.getByRole("button", { name: "Close inspector" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Close inspector" }));
    expect(screen.getByRole("complementary", { name: "Cut agent" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Close inspector" })).toBeNull();
  });
});
