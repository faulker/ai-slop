## Background
A android and macos app that is able to access the audio plug in on the phone or the plugged in USB-C audio input/output device so that it can transmit and receive data in an audio format. The data will be encrypted/decrypted using a string as the encryption key so as long as the device receiving and attempting to decrypt the data has the same string encryption key it will be able to decrypt the data. The app would be able to send text or audio encrypted via sound.

## Goal
A cross-platform app that runs on Android and MacOS that is able to encrypt and decrypt data and send/receive data via audio format.

## References
<!-- Links that add context: Linear tickets, PRs, branches, Slack threads, Notion docs, etc. -->

## Role
You are a task manager agent. You do not implement tasks yourself -- you delegate each step
to a subagent, verify its output against the acceptance criteria below, and decide whether
to proceed, retry, or escalate.

## Plan
The following steps must be completed in order:
1. Research if there are any protocols that handle this kind of transmission
2. Find the best way to handle error checking via audio receiving and transmission so that decryption is possible on disconnection or packet loss.
3. Build a plan that can be executed on to build the app
4. Build the app

> If a step's output does not meet the acceptance criteria, retry once with corrective
> guidance. If it fails again, stop and report the failure before proceeding.

## Acceptance Criteria
<!-- How do you know each step succeeded? List checkable conditions. -->
- [ ] A complete plan to build the app
- [ ] A app that can be compiled to run on MacOS and Android

## Output
<!-- What should the agent return when the task is complete? -->
Example: a summary of actions taken, any files modified, and any unresolved questions.

## Constraints
- **Do not make assumptions.** If something is ambiguous or missing, ask before proceeding.
- **Batch clarifying questions.** If you need to ask, collect all questions and ask at once
  before starting any steps.
- **Do not proceed past a failed step** without explicit instruction.
- <!-- Any domain-specific constraints: don't modify production, don't delete data, etc. -->
