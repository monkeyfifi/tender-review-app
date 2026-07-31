import { invoke } from "@tauri-apps/api/core";
import type {
  JobInput,
  LocalEnvironmentStatus,
  RecoverableJobs,
  ModelSettings,
  ModelProviderPreset,
  PreparedJob,
  ReviewStatus,
  SaveModelSettingsInput,
  TestModelConnectionInput,
} from "./types";

export interface BackendApi {
  getModelSettings(): Promise<ModelSettings>;
  getModelProviderPresets(): Promise<ModelProviderPreset[]>;
  saveModelSettings(input: SaveModelSettingsInput): Promise<ModelSettings>;
  prepareJob(input: JobInput): Promise<PreparedJob>;
  listRecoverableJobs(): Promise<RecoverableJobs>;
  clearJob(jobId: string): Promise<void>;
  clearModelKey(): Promise<void>;
  testModelConnection(input: TestModelConnectionInput): Promise<void>;
  runReview(jobId: string): Promise<ReviewStatus>;
  getReviewStatus(jobId: string): Promise<ReviewStatus>;
  openReportFolder(jobId: string): Promise<string>;
  getLocalEnvironmentStatus(): Promise<LocalEnvironmentStatus>;
  openLocalEnvironmentSetup(): Promise<void>;
}

export const tauriApi: BackendApi = {
  getModelSettings: () => invoke<ModelSettings>("get_model_settings"),
  getModelProviderPresets: () => invoke<ModelProviderPreset[]>("get_model_provider_presets"),
  saveModelSettings: (input) =>
    invoke<ModelSettings>("save_model_settings", { input }),
  prepareJob: (input) => invoke<PreparedJob>("prepare_job", { input }),
  listRecoverableJobs: () => invoke<RecoverableJobs>("list_recoverable_jobs"),
  clearJob: (jobId) => invoke<void>("clear_job", { jobId }),
  clearModelKey: () => invoke<void>("clear_model_key"),
  testModelConnection: (input) => invoke<void>("test_model_connection", { input }),
  runReview: (jobId) => invoke<ReviewStatus>("run_review", { jobId }),
  getReviewStatus: (jobId) => invoke<ReviewStatus>("get_review_status", { jobId }),
  openReportFolder: (jobId) => invoke<string>("open_report_folder", { jobId }),
  getLocalEnvironmentStatus: () => invoke<LocalEnvironmentStatus>("get_local_environment_status"),
  openLocalEnvironmentSetup: () => invoke<void>("open_local_environment_setup"),
};
