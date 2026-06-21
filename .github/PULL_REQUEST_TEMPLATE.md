name: Pull request
description: Open a pull request against agentmux
body:
  - type: markdown
    attributes:
      content: |
        Thanks for the contribution. Please fill in the template below.

  - type: input
    id: pr_title
    attributes:
      label: One-line summary
      placeholder: |
        e.g. "Add `agentmux status --json` for scripting"
    validations:
      required: true

  - type: textarea
    id: motivation
    attributes:
      label: Why is this change needed?
      placeholder: |
        What problem does it solve? Link the issue if one exists.
    validations:
      required: true

  - type: textarea
    id: changes
    attributes:
      label: What changed?
      placeholder: |
        Bullet list of the main changes. Mention any breaking changes explicitly.
    validations:
      required: true

  - type: textarea
    id: testing
    attributes:
      label: How was this tested?
      placeholder: |
        - [ ] `cargo fmt -- --check`
        - [ ] `cargo clippy --all-targets -- -D warnings`
        - [ ] `cargo test`
        - [ ] Manual verification (describe below)

        Manual verification:
    validations:
      required: true

  - type: dropdown
    id: breaking
    attributes:
      label: Breaking change?
      options:
        - No
        - Yes (CLI surface)
        - Yes (daemon protocol)
        - Yes (config file format)
    validations:
      required: true

  - type: textarea
    id: deps
    attributes:
      label: New dependencies added?
      placeholder: |
        If yes, justify each (size, maintenance, license, why it's needed).
    validations:
      required: false