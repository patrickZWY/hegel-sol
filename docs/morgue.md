# Morgue

Launch `hegel-sol morgue --root examples` from the repository root. From a checkout,
use `cargo x build --locked` then `target/debug/hegel-sol morgue --root examples`.
Morgue needs Foundry, Python 3, Tk, and a desktop display.

## Begin with the code

Morgue opens with “What part of the code would you like to work on?” Type
“Walk me through ByteMath.average” and press Enter. Morgue opens the matching Solidity
declaration alongside the conversation and sends its source to the configured agent.
You can also name a project-relative file or use **Workspace → Open source**.

The default command is `codex exec --sandbox read-only -`. To use another agent, set
`HEGEL_MORGUE_AGENT` before launch or edit **Workspace → Agent settings**. For Claude
Code use `claude -p --tools "" --output-format text`. Your agent uses its own login
and permissions.

Ask which inputs exercise rounding near 255. Follow up with “Generate a Solidity
property for that range.” The conversation displays the reply, and a fenced Solidity
test opens in **Draft**. Edit it and choose **Save property** to compile and discover it.

## Learn from a run

Once you have started an agent conversation, completing a search or replay sends its
result back to that agent for one review. Morgue retains the earlier questions,
interpretations, and run summaries for this session. Opening an archive adds it to the
evidence available for your next question. Reopening it counts as the same observation.

For example, three successful pushes into a two-slot stack contradict the capacity
property. Whether the product should reject a push, grow the stack, or overwrite an
entry depends on its intended behavior. Discuss that choice, revise the property or
generator, and run the next experiment. You can question the agent's earlier
interpretation using the same evidence.

## Follow a failure

Choose **Workspace → Browse properties**, select `BoundedStackTest.stateful`, and
click **Run**. **Explore trace** opens the failure beside your conversation. Select a
step below the source to jump to it. A mapped failure highlights the reported line.
For a handler, Morgue can highlight its unique declaration in the current checkout.
The location label identifies which kind of evidence you are viewing. Draws can share
the location of their enclosing call. You can scroll through the entire file, select
code, and ask “Why does this push break the invariant?” in the same conversation.

The **Run** tab shows verdicts, execution counts and diagnostics. **Replay selected
failure** repeats a saved blob. **Workspace → Open run archive** reads an earlier
[archive](archives.md). New archives live under the project's `.hegel/runs/`. Replay
uses the matching checkout and toolchain. **Cancel operation** stops an active command.

## Connect a custom agent

Morgue starts the command in the project directory and sends a JSON request on stdin.
It includes the question, conversation history, project file list, active file and
source text. During debugging it also includes the selected run, trace step, source
location, visible line and highlighted text. The adapter returns plain text on stdout.
A fenced `solidity` block containing a test or invariant populates the draft editor.
A nonzero exit displays the adapter's diagnostic. The opening question is local and
the first agent call occurs when you send a message.

Run `cargo x morgue-smoke` on a host with a display to exercise the conversation,
navigation, property generation, and replay workflows with an offline adapter.
