// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import userEvent from "@testing-library/user-event";
import { renderApp, type FilePicker, type RenderedApp } from "../src/app";
import type { BackendApi } from "../src/api";
import type { PreparedJob, SelectedPath } from "../src/types";

const selectedPath = (path: string): SelectedPath => ({
  path,
  name: path.split(/[\\/]/).pop() ?? path,
});

const picker: FilePicker = {
  openTender: vi.fn(),
  openBid: vi.fn(),
  openBlindBid: vi.fn(),
};

const baseApi = (prepareJob: BackendApi["prepareJob"]): BackendApi => ({
  getModelSettings: vi.fn().mockResolvedValue({
    baseUrl: "https://api.example.com/v1",
    model: "reviewer",
    timeoutSeconds: 60,
    apiKeyRemembered: true,
  }),
  saveModelSettings: vi.fn(),
  prepareJob,
  listRecoverableJobs: vi.fn().mockResolvedValue({ jobs: [], errors: [] }),
  clearJob: vi.fn(),
  clearModelKey: vi.fn(),
  getModelProviderPresets: vi.fn().mockResolvedValue([]),
  testModelConnection: vi.fn(),
  runReview: vi.fn().mockResolvedValue({
    jobId: "job-foundation",
    state: "completed",
    stages: {
      tenderReview: "complete",
      bidReview: "complete",
      duplicateCheck: "complete",
      blindBidCheck: "complete",
      report: "complete",
    },
    reportPath: "C:/fixtures/jobs/job-foundation/report/01-商务审核-投标文件1.md",
    reportMarkdown: "# 商务审核-投标文件1\n",
    reportFiles: [
      {
        title: "商务审核-投标文件1",
        kind: "business",
        path: "C:/fixtures/jobs/job-foundation/report/01-商务审核-投标文件1.md",
        markdown: "# 商务审核-投标文件1\n",
      },
    ],
  }),
  getReviewStatus: vi.fn(),
  openReportFolder: vi.fn().mockResolvedValue("C:/fixtures/jobs/job-foundation/report"),
  getLocalEnvironmentStatus: vi.fn().mockResolvedValue({
    state: "ready",
    message: "环境已就绪",
  }),
  openLocalEnvironmentSetup: vi.fn().mockResolvedValue(undefined),
});

const prepared: PreparedJob = {
  jobId: "job-foundation",
  state: "ready",
  errors: [],
  files: [
    {
      displayName: "tender.pdf",
      role: "tender",
      format: "pdf",
      byteSize: 2048,
      sha256: "a".repeat(64),
      blockCount: 18,
    },
    {
      displayName: "bid-one.docx",
      role: "bid",
      format: "docx",
      byteSize: 1024,
      sha256: "b".repeat(64),
      blockCount: 12,
    },
  ],
};

describe("foundation vertical slice", () => {
  let app: RenderedApp;

  beforeEach(async () => {
    document.body.innerHTML = '<main id="app"></main>';
    app = renderApp({
      root: document.querySelector<HTMLElement>("#app")!,
      api: baseApi(vi.fn().mockResolvedValue(prepared)),
      picker,
    });
    await app.addTender(selectedPath("C:/fixtures/tender.pdf"));
    await app.addBid(selectedPath("C:/fixtures/bid-one.docx"));
  });

  it("locks inputs during preflight then presents prepared file block counts", async () => {
    const user = userEvent.setup();
    let resolveJob: ((job: PreparedJob) => void) | undefined;
    app = renderApp({
      root: document.querySelector<HTMLElement>("#app")!,
      api: baseApi(vi.fn(() => new Promise<PreparedJob>((resolve) => { resolveJob = resolve; }))),
      picker,
    });
    await app.addTender(selectedPath("C:/fixtures/tender.pdf"));
    await app.addBid(selectedPath("C:/fixtures/bid-one.docx"));

    await user.click(document.querySelector<HTMLButtonElement>('[data-action="start"]')!);

    expect(document.body.textContent).toContain("正在预检");
    expect(document.querySelector<HTMLButtonElement>('[data-action="tender"]')?.disabled).toBe(true);
    expect(document.querySelector<HTMLButtonElement>('[data-action="add-bid"]')?.disabled).toBe(true);

    resolveJob?.(prepared);
    await vi.waitFor(() => expect(app.getDraft().phase).toBe("ready"));

    expect(document.body.textContent).toContain("准备完成");
    expect(document.body.textContent).toContain("tender.pdf：18 个文本块");
    expect(document.body.textContent).toContain("bid-one.docx：12 个文本块");
    expect(document.querySelector<HTMLButtonElement>('[data-action="run-review"]')?.textContent).toBe("开始审核");
  });

  it("restores editing and surfaces a stable backend error code after rejection", async () => {
    const user = userEvent.setup();
    const failedApi = baseApi(vi.fn().mockRejectedValue({
      code: "unreadableDocument",
      message: "文件无法读取",
    }));
    app = renderApp({
      root: document.querySelector<HTMLElement>("#app")!,
      api: failedApi,
      picker,
    });
    await app.addTender(selectedPath("C:/fixtures/tender.pdf"));
    await app.addBid(selectedPath("C:/fixtures/bid-one.docx"));

    await user.click(document.querySelector<HTMLButtonElement>('[data-action="start"]')!);
    await vi.waitFor(() => expect(app.getDraft().phase).toBe("failed"));

    expect(document.querySelector<HTMLButtonElement>('[data-action="tender"]')?.disabled).toBe(false);
    expect(document.querySelector<HTMLButtonElement>('[data-action="add-bid"]')?.disabled).toBe(false);
    expect(document.body.textContent).toContain("unreadableDocument");
  });

  it("shows completed report after prepared review succeeds", async () => {
    await app.start();
    await app.runReview();

    expect(document.body.textContent).toContain("审核完成");
    expect(document.body.textContent).toContain("结果预览");
    expect(document.querySelector<HTMLButtonElement>('[data-action="open-report-folder"]')?.disabled).toBe(false);
  });
});
