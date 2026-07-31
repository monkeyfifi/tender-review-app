import type { BackendApi } from "./api";
import type {
  JobInput,
  LocalEnvironmentStatus,
  ModelProviderPreset,
  ModelSettings,
  PreparedFile,
  ReviewStatus,
  SaveModelSettingsInput,
  SelectedPath,
} from "./types";

export type TaskPhase = "editing" | "preflight" | "ready" | "reviewing" | "completed" | "failed";

export interface TaskDraft {
  tender: SelectedPath | null;
  bids: Array<{ bid: SelectedPath; blindBid: SelectedPath | null }>;
  apiStatus: "unconfigured" | "configured";
  phase: TaskPhase;
  errors: Array<{ sourceKey: string; sourceName: string; code: string; message: string }>;
  preparedFiles: PreparedFile[];
  jobId: string | null;
  reviewStatus: ReviewStatus | null;
  reportPath: string | null;
  reportMarkdown: string | null;
  selectedReportPath: string | null;
  localEnvironmentStatus: LocalEnvironmentStatus | null;
}

export interface FilePicker {
  openTender(): Promise<SelectedPath | null>;
  openBid(): Promise<SelectedPath | null>;
  openBlindBid(): Promise<SelectedPath | null>;
}

export interface RenderAppOptions {
  root: HTMLElement;
  api: BackendApi;
  picker: FilePicker;
}

export interface RenderedApp {
  addTender(file: SelectedPath): Promise<void>;
  addBid(file: SelectedPath): Promise<void>;
  addBlindBid(index: number, file: SelectedPath): Promise<void>;
  removeBid(index: number): Promise<void>;
  clearBlindBid(index: number): Promise<void>;
  getDraft(): TaskDraft;
  getAddBidButton(): HTMLButtonElement;
  start(): Promise<void>;
  runReview(): Promise<void>;
}

const MAX_BIDS = 4;
const DEFAULT_MODEL_BASE_URL = "https://api.deepseek.com";
const DEFAULT_MODEL_NAME = "deepseek-v4-flash";

export function renderApp({ root, api, picker }: RenderAppOptions): RenderedApp {
  let draft: TaskDraft = {
    tender: null,
    bids: [],
    apiStatus: "unconfigured",
    phase: "editing",
    errors: [],
    preparedFiles: [],
    jobId: null,
    reviewStatus: null,
    reportPath: null,
    reportMarkdown: null,
    selectedReportPath: null,
    localEnvironmentStatus: null,
  };
  let settings: ModelSettings | null = null;
  let modelPresets: ModelProviderPreset[] = [];

  const update = (next: Partial<TaskDraft>) => {
    draft = { ...draft, ...next };
    render();
  };

  const updateModelStatusOnly = (apiStatus: TaskDraft["apiStatus"]) => {
    draft = { ...draft, apiStatus };
    const dot = root.querySelector<HTMLElement>(".status-dot");
    const label = root.querySelector<HTMLElement>("[data-model-status-label]");
    if (dot) dot.className = `status-dot ${apiStatus}`;
    if (label) label.textContent = apiStatus === "configured" ? "模型已配置" : "模型未就绪";
  };

  const updateLocalEnvironmentStatusOnly = (status: LocalEnvironmentStatus) => {
    draft = { ...draft, localEnvironmentStatus: status };
    const ready = status.state === "ready";
    const dot = root.querySelector<HTMLElement>("[data-environment-status-dot]");
    const label = root.querySelector<HTMLElement>("[data-environment-status-label]");
    const message = root.querySelector<HTMLElement>("[data-environment-status-message]");
    if (dot) dot.className = `status-dot ${ready ? "configured" : "unconfigured"}`;
    if (label) label.textContent = ready ? "环境已就绪" : "环境未就绪";
    if (message) message.textContent = status.message;
  };

  const addTender = async (file: SelectedPath) => {
    if (!isInputEditable(draft)) return;
    update({ tender: file, ...inputEditState() });
  };

  const addBid = async (file: SelectedPath) => {
    if (!isInputEditable(draft) || draft.bids.length >= MAX_BIDS) return;
    update({
      bids: [...draft.bids, { bid: file, blindBid: null }],
      ...inputEditState(),
    });
  };

  const addBlindBid = async (index: number, file: SelectedPath) => {
    const bidIndex = index - 1;
    if (!isInputEditable(draft) || !draft.bids[bidIndex]) return;
    update({
      bids: draft.bids.map((item, currentIndex) =>
        currentIndex === bidIndex ? { ...item, blindBid: file } : item,
      ),
      ...inputEditState(),
    });
  };

  const removeBid = async (index: number) => {
    const bidIndex = index - 1;
    if (!isInputEditable(draft) || !draft.bids[bidIndex]) return;
    update({
      bids: draft.bids.filter((_, currentIndex) => currentIndex !== bidIndex),
      ...inputEditState(),
    });
  };

  const clearBlindBid = async (index: number) => {
    const bidIndex = index - 1;
    if (!isInputEditable(draft) || !draft.bids[bidIndex]) return;
    update({
      bids: draft.bids.map((item, currentIndex) =>
        currentIndex === bidIndex ? { ...item, blindBid: null } : item,
      ),
      ...inputEditState(),
    });
  };

  const start = async () => {
    if (!draft.tender || draft.bids.length === 0 || draft.phase === "preflight") return;
    update({ phase: "preflight", errors: [] });
    try {
      const prepared = await api.prepareJob(toJobInput(draft));
      update({
        errors: prepared.errors.map((error) => ({
          sourceKey: error.sourceKey,
          sourceName: error.sourceName,
          code: error.code,
          message: error.message,
        })),
        phase: prepared.state === "failed" ? "failed" : "ready",
        preparedFiles: prepared.files,
        jobId: prepared.jobId,
        reviewStatus: null,
        reportPath: null,
        reportMarkdown: null,
        selectedReportPath: null,
      });
    } catch (error) {
      update({ phase: "failed", errors: normalizeBackendError(error), preparedFiles: [], jobId: null, reviewStatus: null, reportPath: null, reportMarkdown: null, selectedReportPath: null });
    }
  };

  const runReview = async () => {
    if (!draft.jobId || draft.phase !== "ready") return;
    update({ phase: "reviewing", errors: [] });
    try {
      const status = await api.runReview(draft.jobId);
      update({
        phase: status.state === "failed" ? "failed" : "completed",
        reviewStatus: status,
        reportPath: status.reportPath,
        reportMarkdown: status.reportMarkdown,
        selectedReportPath: status.reportFiles[0]?.path ?? status.reportPath,
      });
    } catch (error) {
      update({ phase: "failed", errors: normalizeBackendError(error) });
    }
  };

  const openReportFolder = async () => {
    if (!draft.jobId || draft.phase !== "completed") return;
    try {
      await api.openReportFolder(draft.jobId);
    } catch (error) {
      update({ errors: normalizeBackendError(error) });
    }
  };

  const resetTask = () => {
    update({
      tender: null,
      bids: [],
      ...inputEditState(),
    });
  };

  const openSettings = () => {
    if (!isInputEditable(draft)) return;
    clearSettingsError(root);
    let dialog = root.querySelector<HTMLDialogElement>("#api-settings");
    if (modelPresets.length > 0 && !dialog?.querySelector('option[value="deepSeek"]')) {
      render();
      dialog = root.querySelector<HTMLDialogElement>("#api-settings");
    }
    dialog?.showModal();
    root.querySelector<HTMLInputElement>("#base-url")?.focus();
  };

  const openEnvironmentSettings = () => {
    clearEnvironmentNotice(root);
    root.querySelector<HTMLDialogElement>("#environment-settings")?.showModal();
    void refreshLocalEnvironment();
  };

  const refreshLocalEnvironment = async () => {
    clearEnvironmentNotice(root);
    try {
      updateLocalEnvironmentStatusOnly(await api.getLocalEnvironmentStatus());
    } catch (error) {
      showEnvironmentError(root, error);
    }
  };

  const openEnvironmentSetup = async () => {
    clearEnvironmentNotice(root);
    try {
      await api.openLocalEnvironmentSetup();
      showEnvironmentSuccess(root, "安装脚本已打开；安装完成后请重新检测。");
    } catch (error) {
      showEnvironmentError(root, error);
    }
  };

  const saveSettings = async (event: SubmitEvent) => {
    event.preventDefault();
    if (!isInputEditable(draft)) return;
    const form = event.currentTarget as HTMLFormElement;
    clearSettingsError(root);
    const values = new FormData(form ?? undefined);
    let apiKey = String(values.get("apiKey") ?? "");
    const input: SaveModelSettingsInput = {
      baseUrl: String(values.get("baseUrl") ?? "").trim(),
      model: String(values.get("model") ?? "").trim(),
      timeoutSeconds: Number(values.get("timeoutSeconds") ?? 60),
      apiKey: apiKey || null,
    };
    try {
      settings = await api.saveModelSettings(input);
      const submittedKey = apiKey.length > 0;
      apiKey = "";
      form.reset();
      const remainsConfigured = submittedKey
        || settings.apiKeyRemembered
        || draft.apiStatus === "configured";
      update({ apiStatus: remainsConfigured ? "configured" : "unconfigured" });
      root.querySelector<HTMLDialogElement>("#api-settings")?.close();
    } catch (error) {
      showSettingsError(root, error);
    } finally {
      apiKey = "";
      const keyField = form.elements.namedItem("apiKey");
      if (keyField instanceof HTMLInputElement) keyField.value = "";
    }
  };

  const clearModelKey = async () => {
    clearSettingsError(root);
    try {
      await api.clearModelKey();
      settings = settings ? { ...settings, apiKeyRemembered: false } : settings;
      update({ apiStatus: "unconfigured" });
    } catch (error) {
      showSettingsError(root, error);
    }
  };

  const applyProviderPreset = (event: Event) => {
    const provider = event.currentTarget as HTMLSelectElement;
    const preset = modelPresets.find((item) => item.provider === provider.value);
    if (!preset) return;
    const form = root.querySelector<HTMLFormElement>("#api-settings-form");
    const baseUrl = form?.elements.namedItem("baseUrl");
    const model = form?.elements.namedItem("model");
    if (baseUrl instanceof HTMLInputElement) baseUrl.value = preset.baseUrl;
    if (model instanceof HTMLInputElement) model.value = preset.model;
  };

  const testModelConnection = async () => {
    clearSettingsError(root);
    clearSettingsSuccess(root);
    const form = root.querySelector<HTMLFormElement>("#api-settings-form");
    const values = new FormData(form ?? undefined);
    try {
      await api.testModelConnection({
        baseUrl: String(values.get("baseUrl") ?? "").trim(),
        model: String(values.get("model") ?? "").trim(),
        timeoutSeconds: Number(values.get("timeoutSeconds") ?? 60),
        apiKey: String(values.get("apiKey") ?? ""),
      });
      showSettingsSuccess(root, "连接成功");
      updateModelStatusOnly("configured");
    } catch (error) {
      updateModelStatusOnly("unconfigured");
      showSettingsError(root, error);
    }
  };

  const render = () => {
    root.innerHTML = `
      <section class="task-shell" aria-label="投标文件智能审核任务台">
        <header class="top-bar">
          <div><p class="eyebrow">本地审核工作台</p><h1>投标文件智能审核</h1></div>
          <div class="status-actions">
            <div class="model-status"><span class="status-dot ${draft.apiStatus}"></span><span data-model-status-label>${draft.apiStatus === "configured" ? "模型已配置" : "模型未就绪"}</span><button class="text-button" type="button" data-action="settings" ${!isInputEditable(draft) ? "disabled" : ""}>API 设置</button></div>
            <div class="model-status"><span class="status-dot ${localEnvironmentReady(draft) ? "configured" : "unconfigured"}" data-environment-status-dot></span><span data-environment-status-label>${localEnvironmentReady(draft) ? "环境已就绪" : "环境未就绪"}</span><button class="text-button" type="button" data-action="environment-settings">环境设置</button></div>
          </div>
        </header>
        ${workflowSteps(draft)}
        ${preparedSummary(draft)}
        ${reviewSummary(draft)}
        <main class="intake-grid">
          <section class="file-card tender-card" aria-labelledby="tender-heading">
            <div><p class="section-kicker">必选</p><h2 id="tender-heading">招标文件</h2><p>支持可提取文本的 PDF 或 DOCX。</p></div>
            ${fileSlot(draft.tender, "tender", "选择招标文件", errorsFor(draft.errors, "tender"), !isInputEditable(draft))}
          </section>
          <section class="file-card bids-card" aria-labelledby="bids-heading">
            <div class="section-heading"><div><p class="section-kicker">必选</p><h2 id="bids-heading">投标文件</h2></div><span class="count">${draft.bids.length} / ${MAX_BIDS}</span></div>
            <p>每份主体文件可关联一份技术暗标 DOCX。</p>
            <div class="bid-list">${draft.bids.map((entry, index) => bidRow(entry, index + 1, draft.errors, !isInputEditable(draft))).join("")}</div>
            <button class="secondary-button" type="button" data-action="add-bid" ${draft.bids.length >= MAX_BIDS || !isInputEditable(draft) ? "disabled" : ""}>添加投标文件</button>
          </section>
        </main>
        <section class="local-checks" aria-label="本地检查说明"><h2>开始后将执行</h2><ul><li>文件可读性与格式预检</li><li>招投标材料结构化准备</li><li>后续本地文本查重与技术暗标检查</li></ul><p>原始文件与本地检查结果仅保存在当前电脑；模型仅接收提取文本。</p></section>
        <div class="primary-action">${primaryAction(draft)}${resetAction(draft)}</div>
        ${globalError(draft)}
        <footer class="app-footer">保定移动政企部 zhaoyun_bd@he</footer>
      </section>
      ${settingsDialog(settings, modelPresets, !isInputEditable(draft), draft.apiStatus === "configured")}
      ${environmentDialog(draft.localEnvironmentStatus)}
    `;

    root.querySelector<HTMLButtonElement>('[data-action="settings"]')?.addEventListener("click", openSettings);
    root.querySelector<HTMLButtonElement>('[data-action="environment-settings"]')?.addEventListener("click", openEnvironmentSettings);
    root.querySelector<HTMLButtonElement>('[data-action="cancel-environment-settings"]')?.addEventListener("click", () => root.querySelector<HTMLDialogElement>("#environment-settings")?.close());
    root.querySelector<HTMLButtonElement>('[data-action="recheck-environment"]')?.addEventListener("click", refreshLocalEnvironment);
    root.querySelector<HTMLButtonElement>('[data-action="open-environment-setup"]')?.addEventListener("click", openEnvironmentSetup);
    root.querySelector<HTMLFormElement>("#api-settings-form")?.addEventListener("submit", saveSettings);
    root.querySelector<HTMLButtonElement>('[data-action="cancel-settings"]')?.addEventListener("click", () => root.querySelector<HTMLDialogElement>("#api-settings")?.close());
    root.querySelector<HTMLButtonElement>('[data-action="clear-model-key"]')?.addEventListener("click", clearModelKey);
    root.querySelector<HTMLSelectElement>('[name="provider"]')?.addEventListener("change", applyProviderPreset);
    root.querySelector<HTMLButtonElement>('[data-action="test-model-connection"]')?.addEventListener("click", testModelConnection);
    root.querySelector<HTMLButtonElement>('[data-action="tender"]')?.addEventListener("click", async () => {
      const file = await picker.openTender();
      if (file) await addTender(file);
    });
    root.querySelector<HTMLButtonElement>('[data-action="add-bid"]')?.addEventListener("click", async () => {
      const file = await picker.openBid();
      if (file) await addBid(file);
    });
    root.querySelectorAll<HTMLButtonElement>('[data-action="blind-bid"]').forEach((button) => {
      button.addEventListener("click", async () => {
        const file = await picker.openBlindBid();
        if (file) await addBlindBid(Number(button.dataset.bidIndex), file);
      });
    });
    root.querySelectorAll<HTMLButtonElement>('[data-action="remove-bid"]').forEach((button) => {
      button.addEventListener("click", () => removeBid(Number(button.dataset.bidIndex)));
    });
    root.querySelectorAll<HTMLButtonElement>('[data-action="clear-blind"]').forEach((button) => {
      button.addEventListener("click", () => clearBlindBid(Number(button.dataset.bidIndex)));
    });
    root.querySelector<HTMLButtonElement>('[data-action="start"]')?.addEventListener("click", start);
    root.querySelector<HTMLButtonElement>('[data-action="run-review"]')?.addEventListener("click", runReview);
    root.querySelector<HTMLButtonElement>('[data-action="open-report-folder"]')?.addEventListener("click", openReportFolder);
    root.querySelector<HTMLButtonElement>('[data-action="reset-task"]')?.addEventListener("click", resetTask);
    root.querySelectorAll<HTMLButtonElement>('[data-action="select-report"]').forEach((button) => {
      button.addEventListener("click", () => update({ selectedReportPath: button.dataset.reportPath ?? null }));
    });
  };

  render();
  void api.getModelSettings().then((loaded) => {
    settings = loaded;
    const apiStatus = loaded.apiKeyRemembered ? "configured" : "unconfigured";
    if (root.querySelector<HTMLDialogElement>("#api-settings")?.hasAttribute("open")) {
      updateModelStatusOnly(apiStatus);
    } else {
      update({ apiStatus });
    }
  }).catch(() => undefined);
  void api.getModelProviderPresets().then((loaded) => {
    modelPresets = loaded;
    if (!root.querySelector<HTMLDialogElement>("#api-settings")?.hasAttribute("open")) render();
  }).catch(() => undefined);
  void api.getLocalEnvironmentStatus().then(updateLocalEnvironmentStatusOnly).catch(() => undefined);

  return {
    addTender,
    addBid,
    addBlindBid,
    removeBid,
    clearBlindBid,
    getDraft: () => structuredClone(draft),
    getAddBidButton: () => root.querySelector<HTMLButtonElement>('[data-action="add-bid"]')!,
    start,
    runReview,
  };
}

function toJobInput(draft: TaskDraft): JobInput {
  return {
    tenderPath: draft.tender!.path,
    bids: draft.bids.map(({ bid, blindBid }) => ({ bidPath: bid.path, blindBidPath: blindBid?.path ?? null })),
  };
}

function fileSlot(file: SelectedPath | null, action: string, label: string, errors: TaskDraft["errors"], disabled: boolean): string {
  return `<div class="file-slot">${file ? `<strong>${escapeHtml(file.name)}</strong>` : "<span>尚未选择文件</span>"}<button class="secondary-button" type="button" data-action="${action}" ${disabled ? "disabled" : ""}>${file ? "更换文件" : label}</button>${renderErrors(errors)}</div>`;
}

function bidRow(entry: TaskDraft["bids"][number], index: number, errors: TaskDraft["errors"], disabled: boolean): string {
  const bidKey = `bid:${index - 1}`;
  const blindKey = `blind:${index - 1}`;
  return `<article class="bid-row"><div class="bid-file"><span class="bid-number">${index}</span><div><strong>${escapeHtml(entry.bid.name)}</strong>${renderErrors(errorsFor(errors, bidKey))}</div><button class="text-button remove-button" type="button" data-action="remove-bid" data-bid-index="${index}" ${disabled ? "disabled" : ""}>删除</button></div><div class="blind-bid"><span>技术暗标（可选）</span>${entry.blindBid ? `<strong>${escapeHtml(entry.blindBid.name)}</strong>` : "<span>未关联</span>"}<div><button class="text-button" type="button" data-action="blind-bid" data-bid-index="${index}" ${disabled ? "disabled" : ""}>${entry.blindBid ? "更换暗标" : "关联 DOCX"}</button>${entry.blindBid ? `<button class="text-button" type="button" data-action="clear-blind" data-bid-index="${index}" ${disabled ? "disabled" : ""}>解除关联</button>` : ""}</div>${renderErrors(errorsFor(errors, blindKey))}</div></article>`;
}

function settingsDialog(settings: ModelSettings | null, modelPresets: ModelProviderPreset[], disabled: boolean, hasConfiguredKey: boolean): string {
  const baseUrl = settings?.baseUrl ?? DEFAULT_MODEL_BASE_URL;
  const model = settings?.model ?? DEFAULT_MODEL_NAME;
  const timeout = settings?.timeoutSeconds ?? 60;
  const provider = modelPresets.find((preset) => preset.baseUrl === baseUrl && preset.model === model)?.provider ?? "custom";
  const providerOptions = modelPresets.map((preset) => `<option value="${preset.provider}" ${provider === preset.provider ? "selected" : ""}>DeepSeek</option>`).join("");
  return `<dialog id="api-settings" aria-labelledby="settings-title"><form id="api-settings-form" method="dialog"><header><h2 id="settings-title">API 设置</h2><p>请配置兼容 OpenAI 的模型服务。远程地址必须使用 HTTPS。</p></header><label>服务商<select name="provider" ${disabled ? "disabled" : ""}>${providerOptions}<option value="custom" ${provider === "custom" ? "selected" : ""}>通用大模型配置</option></select></label><label>服务地址<input id="base-url" name="baseUrl" type="url" required value="${escapeAttribute(baseUrl)}" ${disabled ? "disabled" : ""}></label><label>模型名称<input name="model" type="text" required value="${escapeAttribute(model)}" ${disabled ? "disabled" : ""}></label><label>超时（秒）<input name="timeoutSeconds" type="number" min="1" max="600" required value="${timeout}" ${disabled ? "disabled" : ""}></label><label>API Key<input name="apiKey" type="password" autocomplete="off" ${disabled ? "disabled" : ""}></label><p class="privacy-note">密钥仅保存到系统凭据管理器，不会写入任务文件或报告。</p><footer>${hasConfiguredKey ? `<button class="text-button" type="button" data-action="clear-model-key" ${disabled ? "disabled" : ""}>清除当前密钥</button>` : ""}<button class="secondary-button" type="button" data-action="cancel-settings">取消</button><button class="secondary-button" type="button" data-action="test-model-connection" ${disabled ? "disabled" : ""}>测试连接</button><button class="primary-button" type="submit" ${disabled ? "disabled" : ""}>保存设置</button></footer></form></dialog>`;
}

function environmentDialog(status: LocalEnvironmentStatus | null): string {
  const message = status?.message ?? "正在检测本地环境。";
  return `<dialog id="environment-settings" aria-labelledby="environment-settings-title"><form method="dialog"><header><h2 id="environment-settings-title">本地环境设置</h2><p>技术暗标检查需要本机 Python 3 和相关依赖。</p></header><p class="privacy-note" data-environment-status-message>${escapeHtml(message)}</p><footer><button class="secondary-button" type="button" data-action="cancel-environment-settings">关闭</button><button class="secondary-button" type="button" data-action="recheck-environment">重新检测</button><button class="primary-button" type="button" data-action="open-environment-setup">打开安装脚本</button></footer></form></dialog>`;
}

function localEnvironmentReady(draft: TaskDraft): boolean {
  return draft.localEnvironmentStatus?.state === "ready";
}

function errorsFor(errors: TaskDraft["errors"], sourceKey: string): TaskDraft["errors"] {
  return errors.filter((error) => error.sourceKey === sourceKey);
}

function renderErrors(errors: TaskDraft["errors"]): string {
  return errors.map((error) => `<p class="inline-error" data-source-key="${escapeAttribute(error.sourceKey)}">${escapeHtml(error.message)}</p>`).join("");
}

function canStart(draft: TaskDraft): boolean {
  return isInputEditable(draft) && draft.apiStatus === "configured" && draft.tender !== null && draft.bids.length > 0;
}

function sourceExists(draft: TaskDraft, sourceKey: string): boolean {
  if (sourceKey === "tender") return draft.tender !== null;
  const match = /^(bid|blind):(\d+)$/.exec(sourceKey);
  if (!match) return false;
  const bid = draft.bids[Number(match[2])];
  return match[1] === "bid" ? bid !== undefined : bid?.blindBid !== null && bid?.blindBid !== undefined;
}

function globalError(draft: TaskDraft): string {
  if (draft.phase !== "failed") return "";
  const error = draft.errors.find((item) => !sourceExists(draft, item.sourceKey));
  if (!error) return "";
  const message = draft.jobId
    ? "审核未完成，请检查模型返回或稍后重试。"
    : "任务预检未完成，请检查文件后重试。";
  return `<p class="global-error" role="alert">${message}${escapeHtml(error.message)} 错误代码：${escapeHtml(error.code)}</p>`;
}

function normalizeBackendError(error: unknown): TaskDraft["errors"] {
  const candidate = typeof error === "object" && error !== null
    ? error as { code?: string; message?: string; fileErrors?: Array<{ sourceKey?: string; sourceName?: string; code?: string; message?: string }> }
    : {};
  const fileErrors = Array.isArray(candidate.fileErrors)
    ? candidate.fileErrors
      .filter((item) => typeof item.sourceKey === "string")
      .map((item) => ({
        sourceKey: item.sourceKey!,
        sourceName: item.sourceName ?? "所选文件",
        code: item.code ?? "preflightFailed",
        message: item.message ?? "文件预检失败",
      }))
    : [];
  return [
    ...fileErrors,
    {
      sourceKey: "task",
      sourceName: "任务预检",
      code: candidate.code ?? "preflightFailed",
      message: candidate.message ?? "任务预检失败，请稍后重试",
    },
  ];
}

function clearSettingsError(root: HTMLElement): void {
  root.querySelector("[data-settings-error]")?.remove();
}

function clearSettingsSuccess(root: HTMLElement): void {
  root.querySelector("[data-settings-success]")?.remove();
}

function showSettingsSuccess(root: HTMLElement, message: string): void {
  const notice = document.createElement("p");
  notice.dataset.settingsSuccess = "";
  notice.setAttribute("role", "status");
  notice.textContent = message;
  const footer = root.querySelector("#api-settings-form footer");
  if (footer) footer.before(notice);
}

function showSettingsError(root: HTMLElement, error: unknown): void {
  clearSettingsError(root);
  clearSettingsSuccess(root);
  const candidate = typeof error === "object" && error !== null
    ? error as { code?: string; message?: string }
    : {};
  const alert = document.createElement("p");
  alert.dataset.settingsError = "";
  alert.className = "inline-error";
  alert.setAttribute("role", "alert");
  alert.textContent = `${candidate.message ?? "模型设置操作失败"}（错误代码：${candidate.code ?? "settingsOperationFailed"}）`;
  const form = root.querySelector<HTMLFormElement>("#api-settings-form");
  const footer = form?.querySelector("footer");
  if (footer) footer.before(alert);
  else form?.append(alert);
}

function clearEnvironmentNotice(root: HTMLElement): void {
  root.querySelector("[data-environment-error]")?.remove();
  root.querySelector("[data-environment-success]")?.remove();
}

function showEnvironmentSuccess(root: HTMLElement, message: string): void {
  const notice = document.createElement("p");
  notice.dataset.environmentSuccess = "";
  notice.setAttribute("role", "status");
  notice.textContent = message;
  const footer = root.querySelector("#environment-settings footer");
  if (footer) footer.before(notice);
}

function showEnvironmentError(root: HTMLElement, error: unknown): void {
  clearEnvironmentNotice(root);
  const candidate = typeof error === "object" && error !== null
    ? error as { code?: string; message?: string }
    : {};
  const alert = document.createElement("p");
  alert.dataset.environmentError = "";
  alert.className = "inline-error";
  alert.setAttribute("role", "alert");
  alert.textContent = `${candidate.message ?? "环境设置操作失败"}（错误代码：${candidate.code ?? "environmentOperationFailed"}）`;
  const footer = root.querySelector("#environment-settings footer");
  if (footer) footer.before(alert);
}

function isInputEditable(draft: TaskDraft): boolean {
  return draft.phase === "editing" || draft.phase === "failed";
}

function inputEditState(): Pick<TaskDraft, "phase" | "errors" | "preparedFiles" | "jobId" | "reviewStatus" | "reportPath" | "reportMarkdown" | "selectedReportPath"> {
  return { phase: "editing", errors: [], preparedFiles: [], jobId: null, reviewStatus: null, reportPath: null, reportMarkdown: null, selectedReportPath: null };
}

function preparedSummary(draft: TaskDraft): string {
  if (draft.phase === "preflight") {
    return '<section class="prepared-summary" aria-live="polite"><h2>正在预检</h2><p>正在核验文件格式与可提取文本。</p></section>';
  }
  if (draft.phase !== "ready") return "";

  const entries = draft.preparedFiles
    .map((file) => `<li>${escapeHtml(file.displayName)}：${file.blockCount} 个文本块</li>`)
    .join("");
  return `<section class="prepared-summary" aria-live="polite"><h2>准备完成</h2><p>已完成本地文件预检，可以开始审核。</p><ul>${entries}</ul></section>`;
}

function reviewSummary(draft: TaskDraft): string {
  if (draft.phase === "reviewing") {
    return '<section class="prepared-summary review-summary" aria-live="polite"><h2>正在审核</h2><p>正在执行招标要求提取、逐份投标审核、两两查重、技术暗标检查与报告生成。</p></section>';
  }
  if (draft.phase !== "completed" || !draft.reviewStatus) return "";
  const stageRows = Object.entries(draft.reviewStatus.stages)
    .map(([stage, state]) => `<li>${stageLabel(stage)}：${stageStateLabel(String(state))}</li>`)
    .join("");
  const issueText = draft.reviewStatus.state === "completedWithIssues" ? "审核完成，存在需复核项目" : "审核完成";
  const reportFiles = draft.reviewStatus.reportFiles ?? [];
  const selected = reportFiles.find((file) => file.path === draft.selectedReportPath) ?? reportFiles[0];
  const fallbackMarkdown = draft.reportMarkdown ? { title: "审核结果", markdown: draft.reportMarkdown } : null;
  const preview = selected ?? fallbackMarkdown;
  const list = reportFiles.length > 0
    ? `<div class="result-list"><h3>结果列表</h3><div>${reportFiles.map((file) => `<button class="result-item ${file.path === selected?.path ? "active" : ""}" type="button" data-action="select-report" data-report-path="${escapeAttribute(file.path)}" data-report-title="${escapeAttribute(file.title)}">${escapeHtml(file.title)}</button>`).join("")}</div></div>`
    : "";
  return `<section class="prepared-summary review-summary" aria-live="polite"><h2>${issueText}</h2><ul>${stageRows}</ul>${draft.reportPath ? `<p>当前结果文件：${escapeHtml(draft.reportPath)}</p>` : ""}${list}${preview ? `<div class="result-preview"><h3>结果预览：${escapeHtml(preview.title)}</h3><div class="markdown-preview">${renderMarkdown(preview.markdown)}</div></div>` : ""}</section>`;
}

function primaryAction(draft: TaskDraft): string {
  if (draft.phase === "preflight") {
    return '<button class="primary-button" type="button" data-action="start" disabled>正在预检...</button>';
  }
  if (draft.phase === "ready") {
    return '<button class="primary-button" type="button" data-action="run-review">开始审核</button>';
  }
  if (draft.phase === "reviewing") {
    return '<button class="primary-button" type="button" data-action="run-review" disabled>正在审核...</button>';
  }
  if (draft.phase === "completed") {
    return '<button class="primary-button" type="button" data-action="open-report-folder">打开结果目录</button>';
  }
  return `<button class="primary-button" type="button" data-action="start" ${!canStart(draft) ? "disabled" : ""}>开始预检</button>`;
}

function resetAction(draft: TaskDraft): string {
  return draft.phase === "completed" || draft.phase === "failed"
    ? '<button class="secondary-button" type="button" data-action="reset-task">重置任务</button>'
    : "";
}

function workflowSteps(draft: TaskDraft): string {
  const current = draft.phase === "completed"
    ? 3
    : draft.phase === "preflight" || draft.phase === "ready" || draft.phase === "reviewing" || (draft.phase === "failed" && draft.jobId)
      ? 2
      : 1;
  const labels = ["添加文件", "批量审核", "查看结果"];
  return `<ol class="steps" aria-label="审核流程">${labels.map((label, index) => {
    const number = index + 1;
    const state = number < current ? "completed" : number === current ? "active" : "";
    return `<li class="${state}">${number}. ${label}</li>`;
  }).join("")}</ol>`;
}

function stageLabel(stage: string): string {
  return ({
    tenderReview: "招标要求",
    bidReview: "投标审核",
    duplicateCheck: "文本查重",
    blindBidCheck: "技术暗标",
    report: "汇总报告",
    preflight: "文件预检",
    extract: "文本提取",
    modelTest: "模型测试",
  } as Record<string, string>)[stage] ?? stage;
}

function stageStateLabel(state: string): string {
  return ({
    pending: "等待",
    running: "进行中",
    complete: "完成",
    failed: "失败",
    cancelled: "已取消",
  } as Record<string, string>)[state] ?? state;
}

function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (character) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[character]!);
}

function escapeAttribute(value: string): string {
  return escapeHtml(value);
}

function renderMarkdown(markdown: string): string {
  const lines = markdown.split(/\r?\n/);
  const html: string[] = [];
  let paragraph: string[] = [];
  let list: string[] = [];
  let index = 0;

  const flushParagraph = () => {
    if (paragraph.length > 0) {
      html.push(`<p>${paragraph.map(escapeHtml).join("<br>")}</p>`);
      paragraph = [];
    }
  };
  const flushList = () => {
    if (list.length > 0) {
      html.push(`<ul>${list.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}</ul>`);
      list = [];
    }
  };

  while (index < lines.length) {
    const line = lines[index];
    if (!line.trim()) {
      flushParagraph();
      flushList();
      index += 1;
      continue;
    }
    if (isMarkdownTableStart(lines, index)) {
      flushParagraph();
      flushList();
      const tableRows: string[][] = [splitMarkdownTableRow(lines[index])];
      index += 2;
      while (index < lines.length && isMarkdownTableRow(lines[index])) {
        tableRows.push(splitMarkdownTableRow(lines[index]));
        index += 1;
      }
      const [head, ...body] = tableRows;
      html.push(`<div class="table-scroll"><table><thead><tr>${head.map((cell) => `<th>${renderMarkdownCell(cell)}</th>`).join("")}</tr></thead><tbody>${body.map((row) => `<tr>${row.map((cell) => `<td>${renderMarkdownCell(cell)}</td>`).join("")}</tr>`).join("")}</tbody></table></div>`);
      continue;
    }
    if (line.startsWith("# ")) {
      flushParagraph();
      flushList();
      html.push(`<h1>${escapeHtml(line.slice(2).trim())}</h1>`);
    } else if (line.startsWith("## ")) {
      flushParagraph();
      flushList();
      html.push(`<h2>${escapeHtml(line.slice(3).trim())}</h2>`);
    } else if (line.startsWith("### ")) {
      flushParagraph();
      flushList();
      html.push(`<h3>${escapeHtml(line.slice(4).trim())}</h3>`);
    } else if (line.startsWith("- ")) {
      flushParagraph();
      list.push(line.slice(2).trim());
    } else {
      flushList();
      paragraph.push(line.trim());
    }
    index += 1;
  }
  flushParagraph();
  flushList();
  return html.join("");
}

function isMarkdownTableStart(lines: string[], index: number): boolean {
  return isMarkdownTableRow(lines[index]) && index + 1 < lines.length && /^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)+\|?\s*$/.test(lines[index + 1]);
}

function isMarkdownTableRow(line: string): boolean {
  return line.trim().startsWith("|") && line.trim().endsWith("|");
}

function splitMarkdownTableRow(line: string): string[] {
  const trimmed = line.trim().replace(/^\|/, "").replace(/\|$/, "");
  const cells: string[] = [];
  let cell = "";
  let escaped = false;
  for (const character of trimmed) {
    if (escaped) {
      cell += character;
      escaped = false;
    } else if (character === "\\") {
      escaped = true;
    } else if (character === "|") {
      cells.push(cell.trim());
      cell = "";
    } else {
      cell += character;
    }
  }
  cells.push(cell.trim());
  return cells;
}

function renderMarkdownCell(value: string): string {
  return escapeHtml(value).replace(/&lt;br&gt;/g, "<br>");
}
