# Python Dependency Bootstrap Design

## Goal

Provide a standalone Windows batch script that prepares the Python runtime and
the dependencies required by the bundled `word-format-checker` tool.

## Behavior

The script will live beside the application resources and can be copied out or
run from the installed application directory. It will:

1. Locate a usable Python 3 command (`python`, then `py -3`).
2. When Python is unavailable, download the official 64-bit Python installer
   for Windows, install it for the current user, then locate Python again.
3. Install the dependencies listed in the adjacent
   `resources\\word-format-checker\\requirements.txt` using `python -m pip`.
4. Verify that `docx` can be imported.
5. Keep the terminal open and print a clear actionable error on every failure.

## Boundaries

- The script requires network access only when Python or pip packages need to
  be downloaded.
- It does not alter the application source, model configuration, or review
  workflow.
- It uses only Windows built-in batch and PowerShell capabilities; no new
  application dependency is introduced.

## Validation

- A static test checks that the script resolves its own directory, uses the
  bundled requirements file, installs Python only when absent, and imports
  `docx` after installation.
- The installation guide will instruct end users to double-click the script.
