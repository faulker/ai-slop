## Background
<!-- What context does the agent need? What system/codebase/project is this about? -->
There are times when I want to execute multiple cli commands in peralal from one terminal and be abel to see and interact with the output.

## Goal
<!-- The end state. What does "done" look like? Be specific. -->
Create a TUI tool that allows for inputting one or more commands to execute and for each command it opens a tmux instance of the current directory and executes the command. Each tmux instance would be visable as a panel. So 4 commands would be 4 panesl. There would also be a view mode that would just have the command sessions as a list on the left and the when one was selected it would show the tmux instance.

The user would be able to interact with any of the tmux panels and jump into and out of a single one (attach/detach)
They would be able to close a session but the session output should stay open until they close it. If they exit the tool they should be prompted if they want to keep it running or not, if they do then they would be able to execute the tool with a session ID and it would open that session. The tool also needs a switch that will list all active sessions.

At the start of the tool it should make sure the needed tooling is installed, such as tmux, and if not offer to install it or tell them how to install it

## Role
You are a task manager agent. You do not implement tasks yourself -- you delegate each step to a subagent, verify its output against the acceptance criteria below, and decide whether to proceed, retry, or escalate.

## Plan
The following steps must be completed in order:
1. Figure out the best way to work with tmux to fit the needs of the tool
2. Plan out the best way to implament this tool
3. Create a plan
4. Any other steps you think are required

> If a step's output does not meet the acceptance criteria, retry once with corrective 
> guidance. If it fails again, stop and report the failure before proceeding.

## Acceptance Criteria
<!-- How do you know each step succeeded? List checkable conditions. -->
- [ ] A plan has been created
- [ ] The tool has been created and is working

## Constraints
- **Do not make assumptions.** If something is ambiguous or missing, ask before proceeding.
- **Batch clarifying questions.** If you need to ask, collect all questions and ask at once 
  before starting any steps.
- **Do not proceed past a failed step** without explicit instruction.
- <!-- Any domain-specific constraints: don't modify production, don't delete data, etc. -->
