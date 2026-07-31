// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderApp } from "../src/app";
import type { BackendApi } from "../src/api";
import type { FilePicker, RenderedApp } from "../src/app";
import type { LocalEnvironmentStatus, ModelProviderPreset, ModelSettings, PreparedJob, SelectedPath } from "../src/types";

const selectedPath = (path: string): SelectedPath => ({
  path,
  name: path.split(/[\\/]/).pop() ?? path,
});

const preparedJob = (errors: PreparedJob["errors"] = []): PreparedJob => ({
  jobId: "job-1",
  state: errors.length === 0 ? "ready" : "readyWithErrors",
  files: [],
  errors,
});

const fakeApi = (errors: PreparedJob["errors"] = []): BackendApi => ({
  getModelSettings: vi.fn().mockResolvedValue({
    baseUrl: "https://api.example.com/v1",
    model: "reviewer",
    timeoutSeconds: 60,
    apiKeyRemembered: true,
  }),
  saveModelSettings: vi.fn(),
  prepareJob: vi.fn().mockResolvedValue(preparedJob(errors)),
  listRecoverableJobs: vi.fn().mockResolvedValue({ jobs: [], errors: [] }),
  clearJob: vi.fn(),
  clearModelKey: vi.fn(),
  runReview: vi.fn().mockResolvedValue({
    jobId: "job-1",
    state: "completed",
    stages: {
      tenderReview: "complete",
      bidReview: "complete",
      duplicateCheck: "complete",
      blindBidCheck: "complete",
      report: "complete",
    },
    reportPath: "C:/fixtures/jobs/job-1/report/01-商务审核-投标文件1.md",
    reportMarkdown: "# 商务审核-投标文件1\n\n| ID | 类别 |\n| --- | --- |\n| R1 | 证明材料 |\n",
    reportFiles: [
      {
        title: "商务审核-投标文件1",
        kind: "business",
        path: "C:/fixtures/jobs/job-1/report/01-商务审核-投标文件1.md",
        markdown: "# 商务审核-投标文件1\n\n| ID | 类别 |\n| --- | --- |\n| R1 | 证明材料 |\n",
      },
      {
        title: "技术文件比对",
        kind: "comparison",
        path: "C:/fixtures/jobs/job-1/report/90-技术文件比对.md",
        markdown: "# 技术文件比对\n\n## 检查范围\n\n技术暗标共同段\n",
      },
    ],
  }),
  getReviewStatus: vi.fn(),
  openReportFolder: vi.fn().mockResolvedValue("C:/fixtures/jobs/job-1/report"),
  getModelProviderPresets: vi.fn().mockResolvedValue([
    { provider: "deepSeek", baseUrl: "https://api.deepseek.com", model: "deepseek-v4-flash" },
  ]),
  testModelConnection: vi.fn().mockResolvedValue(undefined),
  getLocalEnvironmentStatus: vi.fn().mockResolvedValue({
    state: "ready",
    message: "环境已就绪",
  } satisfies LocalEnvironmentStatus),
  openLocalEnvironmentSetup: vi.fn().mockResolvedValue(undefined),
});

const picker = (): FilePicker => ({
  openTender: vi.fn(),
  openBid: vi.fn(),
  openBlindBid: vi.fn(),
});

describe("single-page batch intake", () => {
  let app: RenderedApp;

  beforeEach(() => {
    HTMLDialogElement.prototype.showModal = vi.fn(function (this: HTMLDialogElement) {
      this.setAttribute("open", "");
    });
    HTMLDialogElement.prototype.close = vi.fn(function (this: HTMLDialogElement) {
      this.removeAttribute("open");
    });
    document.body.innerHTML = '<main id="app"></main>';
    app = renderApp({
      root: document.querySelector<HTMLElement>("#app")!,
      api: fakeApi(),
      picker: picker(),
    });
  });

  it("keeps one blind-bid file attached to its bid and disables the fifth bid", async () => {
    await app.addTender(selectedPath("C:/fixtures/tender.pdf"));
    for (let index = 1; index <= 4; index += 1) {
      await app.addBid(selectedPath(`C:/fixtures/bid-${index}.pdf`));
    }
    await app.addBlindBid(1, selectedPath("C:/fixtures/blind-1.docx"));

    expect(app.getDraft().bids[0].blindBid?.name).toBe("blind-1.docx");
    expect(app.getAddBidButton().disabled).toBe(true);
  });

  it("places unsupported and scanned-file errors beside their exact source files", async () => {
    const errors: PreparedJob["errors"] = [
      {
        sourceName: "legacy.doc",
        sourceKey: "bid:0",
        code: "unsupportedExtension",
        message: "仅支持 PDF 或 DOCX 文件",
      },
      {
        sourceName: "scan.pdf",
        sourceKey: "bid:1",
        code: "textNotExtractable",
        message: "未能提取可审核文本",
      },
    ];
    app = renderApp({
      root: document.querySelector<HTMLElement>("#app")!,
      api: fakeApi(errors),
      picker: picker(),
    });
    await app.addTender(selectedPath("C:/fixtures/tender.pdf"));
    await app.addBid(selectedPath("C:/fixtures/legacy.doc"));
    await app.addBid(selectedPath("C:/fixtures/scan.pdf"));

    await app.start();

    expect(
      document.querySelector('[data-source-key="bid:0"]')?.textContent,
    ).toContain("仅支持 PDF 或 DOCX 文件");
    expect(
      document.querySelector('[data-source-key="bid:1"]')?.textContent,
    ).toContain("未能提取可审核文本");
    expect(document.querySelector("[role=alert]")).toBeNull();
  });

  it("removes a bid with its blind bid and keeps remaining rows in order", async () => {
    await app.addBid(selectedPath("C:/fixtures/bid-1.pdf"));
    await app.addBid(selectedPath("C:/fixtures/bid-2.pdf"));
    await app.addBid(selectedPath("C:/fixtures/bid-3.pdf"));
    await app.addBlindBid(2, selectedPath("C:/fixtures/blind-2.docx"));

    await app.removeBid(2);

    expect(app.getDraft().bids.map(({ bid }) => bid.name)).toEqual([
      "bid-1.pdf",
      "bid-3.pdf",
    ]);
    expect(app.getDraft().bids[1].blindBid).toBeNull();
    await app.addBlindBid(2, selectedPath("C:/fixtures/blind-3.docx"));
    await app.clearBlindBid(2);
    expect(app.getDraft().bids[1].blindBid).toBeNull();
  });

  it("freezes every file mutation while prepareJob is pending", async () => {
    let resolveJob: ((job: PreparedJob) => void) | undefined;
    const api = fakeApi();
    api.prepareJob = vi.fn(
      () => new Promise<PreparedJob>((resolve) => { resolveJob = resolve; }),
    );
    app = renderApp({
      root: document.querySelector<HTMLElement>("#app")!,
      api,
      picker: picker(),
    });
    await app.addTender(selectedPath("C:/fixtures/tender.pdf"));
    await app.addBid(selectedPath("C:/fixtures/bid-1.pdf"));
    await app.addBlindBid(1, selectedPath("C:/fixtures/blind-1.docx"));

    const pending = app.start();

    expect(app.getDraft().phase).toBe("preflight");
    for (const action of ["settings", "tender", "add-bid", "blind-bid", "clear-blind", "remove-bid"]) {
      expect(document.querySelector<HTMLButtonElement>(`[data-action="${action}"]`)?.disabled).toBe(true);
    }
    await app.addTender(selectedPath("C:/fixtures/replaced-tender.pdf"));
    await app.addBid(selectedPath("C:/fixtures/bid-2.pdf"));
    await app.addBlindBid(1, selectedPath("C:/fixtures/replaced-blind.docx"));
    await app.clearBlindBid(1);
    await app.removeBid(1);
    expect(app.getDraft().tender?.name).toBe("tender.pdf");
    expect(app.getDraft().bids).toHaveLength(1);
    expect(app.getDraft().bids[0].blindBid?.name).toBe("blind-1.docx");

    resolveJob?.(preparedJob());
    await pending;
    expect(app.getDraft().phase).toBe("ready");
  });

  it("attaches an error to only the matching source key when bid names collide", async () => {
    app = renderApp({
      root: document.querySelector<HTMLElement>("#app")!,
      api: fakeApi([{
        sourceName: "proposal.pdf",
        sourceKey: "bid:1",
        code: "textNotExtractable",
        message: "未能提取可审核文本",
      }]),
      picker: picker(),
    });
    await app.addTender(selectedPath("C:/fixtures/tender.pdf"));
    await app.addBid(selectedPath("C:/alpha/proposal.pdf"));
    await app.addBid(selectedPath("D:/beta/proposal.pdf"));

    await app.start();

    expect(document.querySelector('[data-source-key="bid:0"]')).toBeNull();
    expect(document.querySelector('[data-source-key="bid:1"]')?.textContent).toContain("未能提取可审核文本");
    expect(document.body.textContent).not.toContain("C:/alpha");
    expect(document.body.textContent).not.toContain("D:/beta");
  });

  it("restores editing after a deferred rejected response and clears stale file errors on the next change", async () => {
    let rejectJob: ((error: unknown) => void) | undefined;
    const api = fakeApi();
    api.prepareJob = vi.fn(
      () => new Promise<PreparedJob>((_resolve, reject) => { rejectJob = reject; }),
    );
    app = renderApp({
      root: document.querySelector<HTMLElement>("#app")!,
      api,
      picker: picker(),
    });
    await app.addTender(selectedPath("C:/fixtures/tender.pdf"));
    await app.addBid(selectedPath("C:/fixtures/proposal.pdf"));
    await app.addBlindBid(1, selectedPath("C:/fixtures/blind.docx"));

    const pending = app.start();
    rejectJob?.({
      code: "noReadableBids",
      message: "全部投标文件均无法读取",
      fileErrors: [{
        sourceKey: "bid:0",
        sourceName: "proposal.pdf",
        code: "textNotExtractable",
        message: "未能提取可审核文本",
      }],
    });
    await pending;

    expect(app.getDraft().phase).toBe("failed");
    for (const action of ["settings", "tender", "add-bid", "blind-bid", "clear-blind", "remove-bid"]) {
      expect(document.querySelector<HTMLButtonElement>(`[data-action="${action}"]`)?.disabled).toBe(false);
    }
    expect(document.querySelector('[data-source-key="bid:0"]')).not.toBeNull();

    await app.addTender(selectedPath("C:/fixtures/replacement-tender.pdf"));

    expect(app.getDraft().phase).toBe("editing");
    expect(app.getDraft().errors).toEqual([]);
    expect(document.querySelector('[data-source-key="bid:0"]')).toBeNull();
    expect(app.getDraft().tender?.name).toBe("replacement-tender.pdf");
  });

  it("restores file and settings controls after a rejected prepareJob call", async () => {
    const api = fakeApi();
    api.prepareJob = vi.fn().mockRejectedValue({ code: "unreadableDocument", message: "读取失败" });
    app = renderApp({
      root: document.querySelector<HTMLElement>("#app")!,
      api,
      picker: picker(),
    });
    await app.addTender(selectedPath("C:/fixtures/tender.pdf"));
    await app.addBid(selectedPath("C:/fixtures/proposal.pdf"));

    await app.start();

    expect(app.getDraft().phase).toBe("failed");
    expect(document.querySelector<HTMLButtonElement>('[data-action="settings"]')?.disabled).toBe(false);
    expect(document.querySelector<HTMLButtonElement>('[data-action="tender"]')?.disabled).toBe(false);
    expect(document.querySelector<HTMLButtonElement>('[data-action="add-bid"]')?.disabled).toBe(false);
    await app.addBid(selectedPath("C:/fixtures/second.pdf"));
    expect(app.getDraft().phase).toBe("editing");
    expect(app.getDraft().errors).toEqual([]);
    expect(app.getDraft().bids).toHaveLength(2);
  });

  it("maps rejected per-file errors by source key even when file names collide", async () => {
    const api = fakeApi();
    api.prepareJob = vi.fn().mockRejectedValue({
      code: "noReadableBids",
      message: "全部投标文件均无法读取",
      fileErrors: [{
        sourceKey: "bid:1",
        sourceName: "proposal.pdf",
        code: "textNotExtractable",
        message: "文档不包含可提取文本",
      }],
    });
    app = renderApp({ root: document.querySelector<HTMLElement>("#app")!, api, picker: picker() });
    await app.addTender(selectedPath("C:/fixtures/tender.pdf"));
    await app.addBid(selectedPath("C:/alpha/proposal.pdf"));
    await app.addBid(selectedPath("D:/beta/proposal.pdf"));

    await app.start();

    expect(document.querySelector('[data-source-key="bid:0"]')).toBeNull();
    expect(document.querySelector('[data-source-key="bid:1"]')?.textContent).toContain("文档不包含可提取文本");
    expect(document.querySelector("[role=alert]")?.textContent).toContain("noReadableBids");
    expect(document.body.textContent).not.toContain("C:/alpha");
    expect(document.body.textContent).not.toContain("D:/beta");
  });

  it("saves API keys persistently without a remember-key option", async () => {
    const api = fakeApi();
    api.saveModelSettings = vi.fn().mockResolvedValue({
      baseUrl: "https://api.example.com/v1",
      model: "changed-model",
      timeoutSeconds: 90,
      apiKeyRemembered: true,
    });
    app = renderApp({ root: document.querySelector<HTMLElement>("#app")!, api, picker: picker() });
    document.querySelector<HTMLButtonElement>('[data-action="settings"]')?.click();
    document.querySelector<HTMLInputElement>('[name="apiKey"]')!.value = "saved-key";
    const model = document.querySelector<HTMLInputElement>('[name="model"]')!;
    model.value = "changed-model";
    document.querySelector<HTMLFormElement>("#api-settings-form")?.dispatchEvent(
      new SubmitEvent("submit", { bubbles: true, cancelable: true }),
    );
    await vi.waitFor(() => expect(api.saveModelSettings).toHaveBeenCalled());

    expect(api.saveModelSettings).toHaveBeenCalledWith(expect.objectContaining({
      apiKey: "saved-key",
    }));
    expect(document.querySelector('[name="rememberKey"]')).toBeNull();
  });

  it("offers an explicit clear-key action", async () => {
    const api = fakeApi();
    app = renderApp({ root: document.querySelector<HTMLElement>("#app")!, api, picker: picker() });
    await vi.waitFor(() => expect(document.body.textContent).toContain("模型已配置"));
    document.querySelector<HTMLButtonElement>('[data-action="settings"]')?.click();

    document.querySelector<HTMLButtonElement>('[data-action="clear-model-key"]')?.click();

    await vi.waitFor(() => expect(api.clearModelKey).toHaveBeenCalledOnce());
    expect(document.querySelector('[name="rememberKey"]')).toBeNull();
  });

  it("applies a provider preset and lets the user test the saved connection", async () => {
    const api = fakeApi();
    app = renderApp({ root: document.querySelector<HTMLElement>("#app")!, api, picker: picker() });
    await vi.waitFor(() => expect(api.getModelProviderPresets).toHaveBeenCalledOnce());
    document.querySelector<HTMLButtonElement>('[data-action="settings"]')?.click();

    const provider = document.querySelector<HTMLSelectElement>('[name="provider"]')!;
    expect(provider.textContent).toContain("DeepSeek");
    expect(provider.textContent).toContain("通用大模型配置");
    expect(provider.textContent).not.toContain("豆包");
    expect(provider.textContent).not.toContain("GLM");
    provider.value = "deepSeek";
    provider.dispatchEvent(new Event("change", { bubbles: true }));
    expect(document.querySelector<HTMLInputElement>('[name="baseUrl"]')?.value).toBe("https://api.deepseek.com");
    expect(document.querySelector<HTMLInputElement>('[name="model"]')?.value).toBe("deepseek-v4-flash");
    document.querySelector<HTMLInputElement>('[name="apiKey"]')!.value = "unsaved-key";
    document.querySelector<HTMLButtonElement>('[data-action="test-model-connection"]')?.click();

    await vi.waitFor(() => expect(api.testModelConnection).toHaveBeenCalledWith({
      baseUrl: "https://api.deepseek.com",
      model: "deepseek-v4-flash",
      timeoutSeconds: 60,
      apiKey: "unsaved-key",
    }));
    expect(api.saveModelSettings).not.toHaveBeenCalled();
    expect(document.querySelector("[data-settings-success]")?.textContent).toContain("连接成功");
  });

  it("uses current DeepSeek defaults before saved settings finish loading", () => {
    const api = fakeApi();
    api.getModelSettings = vi.fn(() => new Promise<ModelSettings>(() => undefined));
    app = renderApp({ root: document.querySelector<HTMLElement>("#app")!, api, picker: picker() });

    document.querySelector<HTMLButtonElement>('[data-action="settings"]')?.click();

    expect(document.querySelector<HTMLInputElement>('[name="baseUrl"]')?.value).toBe("https://api.deepseek.com");
    expect(document.querySelector<HTMLInputElement>('[name="model"]')?.value).toBe("deepseek-v4-flash");
  });

  it("shows a stable backend error when testing the connection fails", async () => {
    const api = fakeApi();
    api.testModelConnection = vi.fn().mockRejectedValue({
      code: "modelConnectionTimeout",
      message: "模型服务连接失败",
    });
    app = renderApp({ root: document.querySelector<HTMLElement>("#app")!, api, picker: picker() });
    document.querySelector<HTMLButtonElement>('[data-action="settings"]')?.click();
    document.querySelector<HTMLButtonElement>('[data-action="test-model-connection"]')?.click();

    await vi.waitFor(() => expect(document.querySelector("[data-settings-error]")?.textContent).toContain("modelConnectionTimeout"));
  });

  it("keeps unsaved settings open while provider presets finish loading", async () => {
    let resolvePresets: ((presets: ModelProviderPreset[]) => void) | undefined;
    const api = fakeApi();
    api.getModelProviderPresets = vi.fn(() => new Promise<ModelProviderPreset[]>((resolve) => { resolvePresets = resolve; }));
    app = renderApp({ root: document.querySelector<HTMLElement>("#app")!, api, picker: picker() });
    document.querySelector<HTMLButtonElement>('[data-action="settings"]')?.click();
    const baseUrl = document.querySelector<HTMLInputElement>('[name="baseUrl"]')!;
    baseUrl.value = "https://unsaved.example.com/v1";

    resolvePresets?.([{ provider: "deepSeek", baseUrl: "https://api.deepseek.com", model: "deepseek-v4-flash" }]);
    await vi.waitFor(() => expect(api.getModelProviderPresets).toHaveBeenCalledOnce());

    expect(document.querySelector<HTMLDialogElement>("#api-settings")?.hasAttribute("open")).toBe(true);
    expect(document.querySelector<HTMLInputElement>('[name="baseUrl"]')?.value).toBe("https://unsaved.example.com/v1");
  });

  it("shows local environment status and opens the setup dialog actions", async () => {
    const api = fakeApi();
    api.getLocalEnvironmentStatus = vi
      .fn()
      .mockResolvedValueOnce({ state: "missingDependency", message: "缺少技术暗标检查依赖" })
      .mockResolvedValueOnce({ state: "ready", message: "环境已就绪" });
    app = renderApp({ root: document.querySelector<HTMLElement>("#app")!, api, picker: picker() });

    await vi.waitFor(() => expect(document.body.textContent).toContain("环境未就绪"));
    document.querySelector<HTMLButtonElement>('[data-action="environment-settings"]')?.click();
    expect(document.querySelector<HTMLDialogElement>("#environment-settings")?.hasAttribute("open")).toBe(true);
    expect(document.body.textContent).toContain("缺少技术暗标检查依赖");

    document.querySelector<HTMLButtonElement>('[data-action="recheck-environment"]')?.click();
    await vi.waitFor(() => expect(document.body.textContent).toContain("环境已就绪"));
    document.querySelector<HTMLButtonElement>('[data-action="open-environment-setup"]')?.click();

    await vi.waitFor(() => expect(api.openLocalEnvironmentSetup).toHaveBeenCalledOnce());
  });

  it("offers explicit clear after saving a key", async () => {
    const api = fakeApi();
    api.getModelSettings = vi.fn().mockResolvedValue({
      baseUrl: "https://api.example.com/v1",
      model: "reviewer",
      timeoutSeconds: 60,
      apiKeyRemembered: true,
    });
    api.saveModelSettings = vi.fn().mockResolvedValue({
      baseUrl: "https://api.example.com/v1",
      model: "reviewer",
      timeoutSeconds: 60,
      apiKeyRemembered: true,
    });
    app = renderApp({ root: document.querySelector<HTMLElement>("#app")!, api, picker: picker() });
    const key = document.querySelector<HTMLInputElement>('[name="apiKey"]')!;
    key.value = "saved-secret";

    document.querySelector<HTMLFormElement>("#api-settings-form")?.dispatchEvent(
      new SubmitEvent("submit", { bubbles: true, cancelable: true }),
    );
    await vi.waitFor(() => expect(api.saveModelSettings).toHaveBeenCalled());

    expect(document.querySelector<HTMLButtonElement>('[data-action="clear-model-key"]')).not.toBeNull();
  });

  it("keeps a saved key configured when only the model changes", async () => {
    const api = fakeApi();
    api.getModelSettings = vi.fn().mockResolvedValue({
      baseUrl: "https://api.example.com/v1",
      model: "reviewer",
      timeoutSeconds: 60,
      apiKeyRemembered: true,
    });
    api.saveModelSettings = vi.fn().mockResolvedValue({
      baseUrl: "https://api.example.com/v1",
      model: "reviewer-two",
      timeoutSeconds: 60,
      apiKeyRemembered: true,
    });
    app = renderApp({ root: document.querySelector<HTMLElement>("#app")!, api, picker: picker() });
    document.querySelector<HTMLButtonElement>('[data-action="settings"]')?.click();
    document.querySelector<HTMLInputElement>('[name="apiKey"]')!.value = "saved-secret";
    document.querySelector<HTMLFormElement>("#api-settings-form")?.dispatchEvent(
      new SubmitEvent("submit", { bubbles: true, cancelable: true }),
    );
    await vi.waitFor(() => expect(api.saveModelSettings).toHaveBeenCalledTimes(1));
    document.querySelector<HTMLButtonElement>('[data-action="settings"]')?.click();
    document.querySelector<HTMLInputElement>('[name="model"]')!.value = "reviewer-two";

    document.querySelector<HTMLFormElement>("#api-settings-form")?.dispatchEvent(
      new SubmitEvent("submit", { bubbles: true, cancelable: true }),
    );
    await vi.waitFor(() => expect(api.saveModelSettings).toHaveBeenCalledTimes(2));

    expect(api.saveModelSettings).toHaveBeenLastCalledWith(expect.objectContaining({
      apiKey: null,
    }));
    expect(document.body.textContent).toContain("模型已配置");
    expect(document.querySelector('[data-action="clear-model-key"]')).not.toBeNull();
  });

  it("keeps settings open and shows a stable error when save is rejected", async () => {
    const api = fakeApi();
    api.saveModelSettings = vi.fn().mockRejectedValue({
      code: "configurationPersistenceFailed",
      message: "无法保存模型设置",
    });
    app = renderApp({ root: document.querySelector<HTMLElement>("#app")!, api, picker: picker() });
    document.querySelector<HTMLButtonElement>('[data-action="settings"]')?.click();

    document.querySelector<HTMLFormElement>("#api-settings-form")?.dispatchEvent(
      new SubmitEvent("submit", { bubbles: true, cancelable: true }),
    );

    await vi.waitFor(() => expect(document.querySelector('[data-settings-error][role="alert"]')?.textContent).toContain("configurationPersistenceFailed"));
    expect(document.querySelector('[data-settings-error][role="alert"]')?.textContent).toContain("无法保存模型设置");
    expect(document.querySelector<HTMLDialogElement>("#api-settings")?.hasAttribute("open")).toBe(true);
    expect(document.querySelector<HTMLInputElement>('[name="model"]')?.disabled).toBe(false);
  });

  it("keeps settings open and configured when clear-key is rejected", async () => {
    const api = fakeApi();
    api.clearModelKey = vi.fn().mockRejectedValue({
      code: "credentialUnavailable",
      message: "无法访问模型凭据",
    });
    app = renderApp({ root: document.querySelector<HTMLElement>("#app")!, api, picker: picker() });
    await vi.waitFor(() => expect(document.body.textContent).toContain("模型已配置"));
    document.querySelector<HTMLButtonElement>('[data-action="settings"]')?.click();

    document.querySelector<HTMLButtonElement>('[data-action="clear-model-key"]')?.click();

    await vi.waitFor(() => expect(document.querySelector('[data-settings-error][role="alert"]')?.textContent).toContain("credentialUnavailable"));
    expect(document.querySelector('[data-settings-error][role="alert"]')?.textContent).toContain("无法访问模型凭据");
    expect(document.querySelector<HTMLDialogElement>("#api-settings")?.hasAttribute("open")).toBe(true);
    expect(document.body.textContent).toContain("模型已配置");
    expect(document.querySelector('[data-action="clear-model-key"]')).not.toBeNull();
  });

  it("runs review after preflight and exposes the completed report folder", async () => {
    const api = fakeApi();
    app = renderApp({ root: document.querySelector<HTMLElement>("#app")!, api, picker: picker() });
    await app.addTender(selectedPath("C:/fixtures/tender.pdf"));
    await app.addBid(selectedPath("C:/fixtures/proposal.pdf"));

    await app.start();
    document.querySelector<HTMLButtonElement>('[data-action="run-review"]')?.click();

    await vi.waitFor(() => expect(api.runReview).toHaveBeenCalledWith("job-1"));
    expect(document.body.textContent).toContain("审核完成");
    expect(document.body.textContent).toContain("结果列表");
    expect(document.body.textContent).toContain("结果预览");
    expect(document.body.textContent).toContain("商务审核-投标文件1");
    expect(document.querySelector(".markdown-preview table")?.textContent).toContain("证明材料");
    expect(document.body.textContent).not.toContain("技术暗标共同段");
    document.querySelector<HTMLButtonElement>('[data-report-title="技术文件比对"]')?.click();
    expect(document.querySelector(".markdown-preview")?.textContent).toContain("技术文件比对");
    expect(document.querySelector(".markdown-preview")?.textContent).toContain("技术暗标共同段");
    const openButton = document.querySelector<HTMLButtonElement>('[data-action="open-report-folder"]');
    expect(openButton?.textContent).toBe("打开结果目录");
    expect(openButton?.disabled).toBe(false);

    openButton?.click();

    await vi.waitFor(() => expect(api.openReportFolder).toHaveBeenCalledWith("job-1"));
  });

  it("advances workflow steps and resets the page without deleting completed results", async () => {
    const api = fakeApi();
    app = renderApp({ root: document.querySelector<HTMLElement>("#app")!, api, picker: picker() });
    await app.addTender(selectedPath("C:/fixtures/tender.pdf"));
    await app.addBid(selectedPath("C:/fixtures/proposal.pdf"));

    await app.start();

    expect(document.querySelector(".steps .completed")?.textContent).toContain("添加文件");
    expect(document.querySelector(".steps .active")?.textContent).toContain("批量审核");

    document.querySelector<HTMLButtonElement>('[data-action="run-review"]')?.click();
    await vi.waitFor(() => expect(app.getDraft().phase).toBe("completed"));

    expect(document.querySelector(".steps .active")?.textContent).toContain("查看结果");
    document.querySelector<HTMLButtonElement>('[data-action="reset-task"]')?.click();

    expect(app.getDraft().phase).toBe("editing");
    expect(app.getDraft().tender).toBeNull();
    expect(app.getDraft().bids).toEqual([]);
    expect(api.clearJob).not.toHaveBeenCalled();
  });

  it("shows review failures as review errors after preflight succeeds", async () => {
    const api = fakeApi();
    api.runReview = vi.fn().mockRejectedValue({
      code: "invalidModelResponse",
      message: "模型服务未返回有效 JSON",
    });
    app = renderApp({ root: document.querySelector<HTMLElement>("#app")!, api, picker: picker() });
    await app.addTender(selectedPath("C:/fixtures/tender.pdf"));
    await app.addBid(selectedPath("C:/fixtures/proposal.pdf"));

    await app.start();
    await app.runReview();

    expect(document.body.textContent).toContain("审核未完成");
    expect(document.body.textContent).toContain("invalidModelResponse");
    expect(document.body.textContent).toContain("模型服务未返回有效 JSON");
    expect(document.body.textContent).not.toContain("任务预检未完成");
  });
});
