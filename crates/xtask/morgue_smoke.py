"""Drive the graphical workflow against an isolated project and a stdin agent."""
import importlib.util
import json
from pathlib import Path
import shutil
import sys
import tempfile
import time
import tkinter as tk
from unittest.mock import patch

root = Path.cwd()
spec = importlib.util.spec_from_file_location("morgue", root/"crates/hegel-sol/src/morgue.py")
morgue = importlib.util.module_from_spec(spec)
sys.dont_write_bytecode = True
spec.loader.exec_module(morgue)

with tempfile.TemporaryDirectory() as temporary:
    project = Path(temporary)
    tests = project/"test"
    tests.mkdir()
    shutil.copy(root/"examples/test/Boundaries.t.sol", tests)
    (project/"foundry.toml").write_text('[profile.default]\nsolc_version="0.8.30"\n')
    adapter = project/"agent.py"
    adapter.write_text('''import json, sys
request = json.load(sys.stdin)
assert 'contract ByteMath' in request['source']
if request['mode'] == 'review':
    observation = request['evidence'][request['active_archive']]
    assert observation['results'][0]['status'] == request['run']['status']
    assert request['conversation'] and observation['budget']['step_count']
    print("Review of " + observation['mode'] + ": " + request['run']['status'])
elif not request['conversation']:
    print("Exercise the range 240 through 255.")
else:
    assert '240' in request['conversation'][0]['answer']
    assert request['selected_step'] and request['run']['origin'] and request['source_line']
    print("""```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;
import {ByteMath} from "./Boundaries.t.sol";
contract GeneratedTest is ByteMath {
    function test_top_range(uint8 raw) public pure {
        uint8 a = 240 + raw % 16;
        assert(average(a, 255) == (uint256(a) + 255) / 2);
    }
}
```""")
''')
    window = tk.Tk()
    app = morgue.Morgue(window,str(root/"target/debug/hegel-sol"),project)
    errors = []
    app.error = errors.append

    def wait():
        deadline = time.monotonic()+90
        while app.busy and time.monotonic() < deadline:
            window.update()
            time.sleep(.02)
        window.update()
        assert not app.busy and not errors, errors

    wait()
    assert len(app.workspace.panes()) == 1
    assert "What part of the code" in app.chat.get("1.0","end")
    app.command.set(f'{sys.executable} "{adapter}"')
    app.question.insert(0,"Explain BoundedStackTest")
    app.ask()
    wait()
    assert app.source_path == tests/"Boundaries.t.sol"
    assert "Exercise the range" in app.chat.get("1.0","end")
    app.browse()
    wait()
    app.choice.set("BoundedStackTest.stateful")
    app.run()
    wait()
    first = app.archive
    failure = next(iter(app.records.values()))
    assert failure["status"] == "failed"
    assert "rule_push" in app.detail.get("1.0","end")
    app.replay()
    wait()
    assert next(iter(app.records.values()))["trace"] == failure["trace"]
    with patch.object(morgue.filedialog,"askdirectory",return_value=str(first)):
        app.open_archive()
    wait()
    assert app.archive == first
    app.explore()
    assert app.source_line and "invariant_size" in app.viewer.get(f"{app.source_line}.0",f"{app.source_line}.end")
    app.trace.selection_clear(0,"end")
    app.trace.selection_set(0)
    app.jump()
    assert "rule_push" in app.viewer.get(f"{app.source_line}.0",f"{app.source_line}.end")
    assert app.chat.winfo_viewable()
    app.question.delete(0,"end")
    app.question.insert(0,"Generate that property.")
    app.ask()
    wait()
    assert "function test_top_range" in app.code.get("1.0","end")
    with patch.object(morgue.filedialog,"asksaveasfilename",return_value=str(tests/"Generated.t.sol")):
        app.save()
    wait()
    target = "GeneratedTest.test_top_range"
    assert target in app.choice["values"], app.choice["values"]
    app.choice.set(target)
    app.run()
    wait()
    assert next(iter(app.records.values()))["status"] == "passed"
    assert [turn["answer"] for turn in app.history if turn["mode"]=="review"] == [
        "Review of search: failed","Review of replay: failed","Review of search: passed"]
    assert len(app.evidence) == 3
    app.work("Waiting…",lambda: app.command_result([sys.executable,"-c","import time; time.sleep(60)"],90),lambda _: None)
    deadline = time.monotonic()+5
    while not app.process and time.monotonic() < deadline:
        window.update()
        time.sleep(.01)
    assert app.process
    app.cancel()
    wait()
    assert app.status.get() == "Cancelled"
    window.destroy()
    print("Morgue discovered, failed, replayed, opened, drafted, compiled and passed.")
