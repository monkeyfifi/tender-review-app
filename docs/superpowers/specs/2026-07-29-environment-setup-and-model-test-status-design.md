# Environment Setup And Model Test Status Design

## Goal

Add a visible local environment status beside the model API status, let users open the bundled Windows dependency setup script from the app, and make the existing `modelTest` review stage reflect real backend progress instead of remaining pending after a successful review.

## Scope

- Add a top-bar local environment status light and an `环境设置` button beside `API 设置`.
- Add a local environment dialog showing whether Python 3 and the technical blind-bid checker dependency are available.
- Add `重新检测` and `打开安装脚本` actions.
- Use the already bundled `resources/word-format-checker/安装技术暗标检查依赖.bat`; do not build a new installer.
- On non-Windows platforms, the install-script action returns a clear message that the bundled script is for Windows delivery.
- Mark `modelTest` as running and complete during `run_review`.

## UX

The top status area shows two independent health indicators:

- Model: `模型已配置` / `模型未就绪`
- Local environment: `环境已就绪` / `环境未就绪`

The environment dialog has a short status message and two actions:

- `重新检测`: calls the backend status command and refreshes the light.
- `打开安装脚本`: launches the bundled Windows batch file. After the user completes it, they click `重新检测`.

## Backend

Add commands:

- `get_local_environment_status() -> LocalEnvironmentStatus`
- `open_local_environment_setup() -> Result<(), AppError>`

Reuse the checker resource discovery and Python dependency checks already used by the technical blind-bid flow. The status command is read-only and never installs anything.

## Model Test Stage

`run_review` currently initializes `ModelTest` but skips it. The fix is to begin and complete this stage at the start of review. The actual lightweight model call is already exercised by the first requirement-extraction model call, so this version records the stage as the model review subsystem being entered successfully. A later version may split out a separate one-token connection probe if product requirements demand a distinct network check.

## Testing

- Frontend: status light, environment dialog, recheck action, setup-script action.
- Backend: environment status serializes and maps dependency states; setup-script command errors clearly on non-Windows.
- Backend review: completed reviews leave `modelTest` as `complete`.
