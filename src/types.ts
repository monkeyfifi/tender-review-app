export interface SelectedPath {
  path: string;
  name: string;
}

export interface BidInput {
  bidPath: string;
  blindBidPath: string | null;
}

export interface JobInput {
  tenderPath: string;
  bids: BidInput[];
}

export interface SaveModelSettingsInput {
  baseUrl: string;
  model: string;
  timeoutSeconds: number;
  apiKey: string | null;
}

export interface ModelSettings {
  baseUrl: string;
  model: string;
  timeoutSeconds: number;
  apiKeyRemembered: boolean;
}

export interface ModelProviderPreset {
  provider: "deepSeek";
  baseUrl: string;
  model: string;
}

export type LocalEnvironmentState = "ready" | "missingPython" | "missingDependency" | "missingChecker";

export interface LocalEnvironmentStatus {
  state: LocalEnvironmentState;
  message: string;
}

export interface TestModelConnectionInput {
  baseUrl: string;
  model: string;
  timeoutSeconds: number;
  apiKey: string;
}

export type ErrorCode =
  | "bidCountOutOfRange"
  | "unsupportedExtension"
  | "blindBidMustBeDocx"
  | "duplicateInputFile"
  | "invalidFileSelection"
  | "noReadableBids"
  | "jobPersistenceFailed"
  | "jobInterrupted"
  | "invalidDocx"
  | "unreadableDocument"
  | "textNotExtractable"
  | "encryptedDocument"
  | "insecureRemoteEndpoint"
  | "corruptJobManifest"
  | "invalidJobId"
  | "credentialUnavailable"
  | "configurationPersistenceFailed"
  | "invalidEndpoint"
  | "invalidModelSettings"
  | "modelApiKeyMissing"
  | "modelConnectionHttpFailed"
  | "modelConnectionTimeout"
  | "modelConnectionInvalidResponse"
  | "reportGenerationFailed"
  | "documentChangedDuringRead"
  | "localEnvironmentUnavailable";

export interface FileErrorRecord {
  sourceName: string;
  sourceKey: string;
  code: ErrorCode;
  message: string;
}

export interface BackendError {
  code: ErrorCode | string;
  message: string;
  fileErrors?: FileErrorRecord[];
}

export interface PreparedFile {
  displayName: string;
  role: "tender" | "bid" | "blindBid";
  format: "pdf" | "docx";
  byteSize: number;
  sha256: string;
  blockCount: number;
}

export interface PreparedJob {
  jobId: string;
  state: JobState;
  files: PreparedFile[];
  errors: FileErrorRecord[];
}

export type JobState =
  | "draft"
  | "preparing"
  | "ready"
  | "readyWithErrors"
  | "completed"
  | "completedWithIssues"
  | "failed"
  | "cancelled";

export type JobStage =
  | "preflight"
  | "extract"
  | "modelTest"
  | "tenderReview"
  | "bidReview"
  | "duplicateCheck"
  | "blindBidCheck"
  | "report";

export type StageState = "pending" | "running" | "complete" | "failed" | "cancelled";

export interface ReviewStatus {
  jobId: string;
  state: JobState;
  stages: Partial<Record<JobStage, StageState>>;
  reportPath: string | null;
  reportMarkdown: string | null;
  reportFiles: ReviewReportFile[];
}

export interface ReviewReportFile {
  title: string;
  kind: string;
  path: string;
  markdown: string;
}

export interface JobSummary {
  jobId: string;
  updatedAt: string;
  state: PreparedJob["state"];
  sourceNames: string[];
}

export interface RecoverableJobError {
  jobDisplay: string;
  code: ErrorCode;
  message: string;
}

export interface RecoverableJobs {
  jobs: JobSummary[];
  errors: RecoverableJobError[];
}
