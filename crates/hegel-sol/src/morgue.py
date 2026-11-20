"""Desktop frontend embedded in the hegel-sol executable."""
import json
import os
from pathlib import Path
import queue
import re
import shlex
import signal
import subprocess
import sys
import threading
import tkinter as tk
from tkinter import filedialog, messagebox, ttk
from tkinter.scrolledtext import ScrolledText
import uuid


# The title card takes the hard light; the reading surfaces stay evenly lit.
BLACK = "#101013"
PANEL = "#1c1b20"
CONTROL = "#302e32"
IVORY = "#f4ead6"
MUTED = "#c6b9ab"
RED = "#a3313c"
FOCUS = "#f2bd89"


def cinema_theme(window):
    window.configure(background=BLACK)
    style = ttk.Style(window)
    style.theme_use("clam")
    style.configure(".", background=PANEL, foreground=IVORY, fieldbackground=BLACK,
                    bordercolor="#585254", lightcolor=CONTROL, darkcolor=BLACK,
                    arrowcolor=IVORY, focuscolor=FOCUS, insertcolor=IVORY)
    style.map(".", foreground=[("disabled","#aaa1a0")],
              background=[("active",CONTROL)], bordercolor=[("focus",FOCUS)])
    for widget in ("TButton","TMenubutton","Treeview.Heading"):
        style.configure(widget,background=CONTROL,padding=(8,4))
    for widget in ("TEntry","TCombobox","Treeview"):
        style.configure(widget,background=BLACK,fieldbackground=BLACK,
                        selectbackground="#70404a",selectforeground=IVORY)
        style.map(widget,fieldbackground=[("readonly",BLACK)],
                  foreground=[("selected",IVORY),("readonly",IVORY)],
                  background=[("selected","#70404a")])
    style.map("TNotebook.Tab",background=[("selected",CONTROL)],
              foreground=[("selected",IVORY)])
    for widget in ("Text","Listbox","Menu","Scrollbar"):
        for key, value in (("background",PANEL),("foreground",IVORY),
                           ("selectBackground","#70404a"),("selectForeground",IVORY),
                           ("insertBackground",IVORY),("highlightColor",FOCUS),
                           ("activeBackground",CONTROL),("activeForeground",IVORY)):
            window.option_add(f"*{widget}.{key}",value)


def title_card(canvas, project, compact):
    """An open door casts a blade of light across an impossible staircase."""
    w,h = canvas.winfo_width(),canvas.winfo_height()
    canvas.delete("all")
    x = int(w * .76)
    floor = int(h * .57)
    door = max(16,int(h * .17))
    canvas.create_polygon(x-door,floor,x+door,floor,w,h,w*.38,h,
                          fill="#9e9385",outline="")
    for step in range(1,7):
        y = floor + (h-floor)*(step/7)**1.6
        left = x-door+(w*.38-x+door)*(y-floor)/max(1,h-floor)
        right = x+door+(w-x-door)*(y-floor)/max(1,h-floor)
        canvas.create_polygon(left,y,right,y,right+3,y+step,left-5,y+step,
                              fill=BLACK,outline="")
    canvas.create_rectangle(x-door,h*.13,x+door,floor,fill=IVORY,outline="")
    canvas.create_polygon(x-door,h*.13,x+door*.23,h*.20,x+door*.23,floor*.93,
                          x-door,floor,fill="#373337",outline="")
    canvas.create_polygon(w*.85,0,w,0,w,h,w*.96,h,fill="#292629",outline="")
    canvas.create_line(0,h-2,w,h-2,fill=RED,width=3)
    canvas.create_text(20,h*.47,text="M O R G U E",anchor="w",fill=IVORY,
                       font=("Times",21 if compact else 38,"bold"))
    if not compact:
        canvas.create_text(24,h*.78,text=project,anchor="w",fill=MUTED,
                           font=("TkFixedFont",11))


class Morgue:
    def __init__(self, window, binary, project):
        self.window, self.binary, self.project = window, binary, project
        self.events = queue.Queue()
        self.busy = False
        self.cancelled = threading.Event()
        self.process_lock = threading.Lock()
        self.process = None
        window.protocol("WM_DELETE_WINDOW", self.close)
        self.records = {}
        self.history = []
        self.evidence = {}
        self.saved = None
        self.archive = None
        window.title(f"Morgue  ·  {project.name}")
        window.geometry("1080x740")
        window.minsize(720, 540)
        self.source_path = None
        self.selected_step = None
        self.source_line = None
        self.sources = {}
        # Index first-party declarations so a conversation can begin with a symbol.
        for folder, dirs, files in os.walk(project):
            dirs[:] = sorted(d for d in dirs if not d.startswith(".") and d not in
                             {"lib","node_modules","out","cache","target"})
            for name in sorted(files):
                if name.endswith(".sol"):
                    path = (Path(folder)/name).resolve()
                    if path.is_relative_to(project):
                        try:
                            self.sources[path] = path.read_text()
                        except (OSError, UnicodeError):
                            pass
        cinema_theme(window)
        base = tk.Frame(window, bg=BLACK, padx=14, pady=10)
        base.pack(fill="both", expand=True)
        header = tk.Frame(base,bg=BLACK)
        header.pack(fill="x",pady=(0,8))
        self.card = tk.Canvas(header,bg=BLACK,height=138,width=1,highlightthickness=0)
        self.card.pack(side="left",fill="x",expand=True)
        self.compact = False
        self.card.bind("<Configure>",lambda _:title_card(self.card,project.name,self.compact))
        tools = ttk.Menubutton(header,text="Workspace ▾")
        menu = tk.Menu(tools,tearoff=False)
        tools.configure(menu=menu)
        tools.pack(side="right")
        menu.add_command(label="Browse properties",command=lambda:self.invoke(self.browse))
        menu.add_command(label="Open run archive…",command=lambda:self.invoke(self.open_archive))
        menu.add_command(label="Open source…",command=lambda:self.invoke(self.source))
        menu.add_command(label="Agent settings…",command=self.settings)
        self.workspace = ttk.Panedwindow(base,orient="horizontal")
        self.workspace.pack(fill="both",expand=True)
        agent = tk.Frame(self.workspace,bg=PANEL,padx=16,pady=16)
        self.workspace.add(agent,weight=2)
        self.tabs = ttk.Notebook(self.workspace)
        runs = ttk.Frame(self.tabs,padding=6)
        source = ttk.Frame(self.tabs,padding=6)
        draft = ttk.Frame(self.tabs,padding=6)
        self.tabs.add(runs,text="Run")
        self.tabs.add(source,text="Source")
        self.tabs.add(draft,text="Draft")
        self.source_label = ttk.Label(source,text="")
        self.source_label.pack(anchor="w")
        self.viewer = ScrolledText(source,wrap="none",font=("TkFixedFont",11),state="disabled")
        self.viewer.pack(fill="both",expand=True)
        self.viewer.tag_configure("location",background="#e1ba88",foreground=BLACK)
        self.code = ScrolledText(draft,wrap="none")
        self.code.pack(fill="both",expand=True)
        self.button(draft,"Save property…",self.save)
        bar = ttk.Frame(runs)
        bar.pack(fill="x")
        self.choice = ttk.Combobox(bar, state="readonly", width=42)
        self.choice.pack(side="left", fill="x", expand=True)
        self.button(bar, "Discover", self.discover)
        budgets = ttk.Frame(runs)
        budgets.pack(fill="x",pady=4)
        self.cases = self.field(budgets, "Cases", "100", 6)
        self.seed = self.field(budgets, "Seed", "17", 6)
        self.steps = self.field(budgets, "Steps", "50", 5)
        self.button(budgets, "Run", self.run)
        actions = ttk.Frame(runs)
        actions.pack(fill="x", pady=8)
        self.button(actions, "Open archive…", self.open_archive)
        self.button(actions, "Replay selected failure", self.replay)
        self.summary = tk.StringVar(value="")
        ttk.Label(runs, textvariable=self.summary).pack(anchor="w")
        self.activity = tk.Canvas(runs, height=34, highlightthickness=0, background=PANEL)
        self.activity.pack(fill="x", pady=(0,6))
        self.activity.bind("<Configure>",lambda event: self.chart())
        split = ttk.Panedwindow(runs, orient="vertical")
        split.pack(fill="both", expand=True)
        self.rows = ttk.Treeview(split, columns=("status", "name", "duration"), show="headings", height=5)
        for col, label, width in [("status","Verdict",90),("name","Property",450),("duration","Time (ms)",90)]:
            self.rows.heading(col, text=label)
            self.rows.column(col, width=width)
        split.add(self.rows, weight=1)
        self.rows.tag_configure("failed", foreground="#ffaca9")
        self.rows.tag_configure("passed", foreground="#c6d7c8")
        self.rows.bind("<<TreeviewSelect>>", lambda e: self.inspect())
        self.detail = ScrolledText(split, wrap="word", height=12)
        split.add(self.detail, weight=3)
        self.trace = tk.Listbox(source,height=5,exportselection=False)
        self.trace.pack(fill="x")
        self.trace.bind("<<ListboxSelect>>",lambda _:self.jump())
        self.button(actions,"Explore trace",self.explore)
        self.command = tk.StringVar(value=os.getenv("HEGEL_MORGUE_AGENT","codex exec --sandbox read-only -"))
        self.chat = ScrolledText(agent,wrap="word",bg=PANEL,fg=IVORY,borderwidth=0,
                                 highlightthickness=0,font=("TkDefaultFont",12),padx=12,pady=12,
                                 spacing3=6,width=36)
        self.chat.pack(fill="both",expand=True)
        self.chat.tag_configure("speaker",foreground="#e8a69d",font=("TkDefaultFont",10,"bold"))
        self.chat.tag_configure("solidity",background=BLACK,foreground=IVORY,
                                font=("TkFixedFont",11),lmargin1=12,lmargin2=12,spacing1=4)
        self.chat.tag_configure("opening",font=("Times",23),foreground=IVORY)
        self.say("Morgue","What part of the code would you like to work on?")
        composer = ttk.Frame(agent)
        composer.pack(side="bottom", fill="x", pady=(12,0), before=self.chat)
        self.question = ttk.Entry(composer,font=("TkDefaultFont",12))
        self.question.pack(side="left",fill="x",expand=True,ipady=5)
        self.button(composer,"Send",self.ask)
        self.question.bind("<Return>",lambda _:self.invoke(self.ask))
        self.question.focus_set()
        self.status = tk.StringVar(value="Ready")
        footer = ttk.Frame(base)
        footer.pack(side="bottom", fill="x", pady=(8,0), before=self.workspace)
        self.stop = ttk.Button(footer, text="Cancel operation", command=self.cancel, state="disabled")
        self.stop.pack(side="right")
        ttk.Label(footer, textvariable=self.status, wraplength=780).pack(side="left")
        self.window.after(50, self.poll)

    def say(self,speaker,message):
        self.chat.configure(state="normal")
        self.chat.insert("end",speaker+"\n","speaker")
        for index, part in enumerate(re.split(r"```(?:[a-zA-Z]+)?\n(.*?)```",message,flags=re.S)):
            tag = "solidity" if index % 2 else ("opening" if not self.history and speaker=="Morgue" else ())
            self.chat.insert("end",part,tag)
        self.chat.insert("end","\n\n")
        self.chat.configure(state="disabled")
        self.chat.see("end")

    def compact_card(self):
        if not self.compact:
            self.compact = True
            self.card.configure(height=48)

    def reveal(self,index):
        if str(self.tabs) not in self.workspace.panes():
            self.workspace.add(self.tabs,weight=3)
        self.compact_card()
        self.tabs.select(index)

    def settings(self):
        dialog = tk.Toplevel(self.window)
        dialog.title("Agent settings")
        dialog.configure(background=PANEL)
        ttk.Label(dialog,text="Agent command").pack(anchor="w",padx=12,pady=6)
        entry = ttk.Combobox(dialog,textvariable=self.command,width=44,values=[
            "codex exec --sandbox read-only -",'claude -p --tools "" --output-format text'])
        entry.pack(padx=12,pady=6)
        ttk.Button(dialog,text="Done",command=dialog.destroy).pack(pady=6)

    def browse(self):
        self.reveal(0)
        self.discover()

    def field(self, bar, label, default, width):
        ttk.Label(bar, text=label).pack(side="left", padx=(6,2))
        entry = ttk.Entry(bar, width=width)
        entry.insert(0, default)
        entry.pack(side="left")
        return entry

    def button(self, bar, label, action):
        ttk.Button(bar, text=label, command=lambda: self.invoke(action)).pack(side="left", padx=2)

    def invoke(self, action):
        if self.busy:
            self.status.set("Please wait for the current operation.")
            return
        try:
            action()
        except Exception as exc:
            self.error(exc)

    def error(self, exc):
        self.status.set("Operation failed")
        messagebox.showerror("Morgue", str(exc), parent=self.window)

    def work(self, label, task, done):
        self.busy = True
        self.stop.configure(state="normal")
        self.cancelled.clear()
        self.status.set(label)
        def worker():
            try:
                self.events.put((done, task(), None))
            except Exception as exc:
                self.events.put((done, None, exc))
        threading.Thread(target=worker, daemon=True).start()

    def poll(self):
        try:
            done, result, error = self.events.get_nowait()
            self.busy = False
            self.stop.configure(state="disabled")
            self.status.set("Ready")
            if self.cancelled.is_set():
                self.status.set("Cancelled")
            elif error:
                self.error(error)
            else:
                self.invoke(lambda: done(result))
        except queue.Empty:
            pass
        self.window.after(50, self.poll)

    def cancel(self):
        self.cancelled.set()
        with self.process_lock:
            if self.process:
                self.terminate(self.process)
        self.status.set("Cancelling…")

    def close(self):
        self.cancel()
        self.window.destroy()

    @staticmethod
    def terminate(process):
        if process.poll() is None:
            try:
                os.killpg(process.pid,signal.SIGKILL)
            except ProcessLookupError:
                pass

    def command_result(self, argv, timeout, prompt=None):
        with self.process_lock:
            if self.cancelled.is_set():
                raise ValueError("Cancelled")
            proc = subprocess.Popen(argv,cwd=self.project,stdin=subprocess.PIPE,
                                    stdout=subprocess.PIPE,stderr=subprocess.PIPE,
                                    text=True,start_new_session=True)
            self.process = proc
        try:
            try:
                stdout, stderr = proc.communicate(prompt,timeout=timeout)
            except subprocess.TimeoutExpired:
                self.terminate(proc)
                proc.communicate()
                raise ValueError(f"Command exceeded {timeout} seconds")
            return subprocess.CompletedProcess(argv,proc.returncode,stdout,stderr)
        finally:
            with self.process_lock:
                self.process = None

    def cli(self, args, allowed=(0,)):
        result = self.command_result([self.binary, *map(str,args)],600)
        if result.returncode not in allowed:
            raise ValueError(result.stderr or result.stdout or f"Exit {result.returncode}")
        return result.stdout

    def discover(self, preferred=None):
        def show(names):
            self.choice["values"] = names
            self.choice.set(preferred if preferred in names else next(iter(names),""))
            self.status.set(f"{len(names)} properties discovered")
        self.choice.set("")
        self.choice["values"] = ()
        self.work("Compiling and discovering properties…",
                  lambda: json.loads(self.cli(["test","--list","--json"])), show)

    def run(self):
        name = self.choice.get()
        if not name:
            raise ValueError("Discover and select a property first.")
        values = [int(field.get()) for field in (self.cases,self.seed,self.steps)]
        if min(values) < 0 or values[0] == 0:
            raise ValueError("Enter a positive case count and nonnegative seed and steps.")
        self.execute(["--exact-test",name,
                      "--test-cases",values[0],"--seed",values[1],"--step-count",values[2]])

    def execute(self, args):
        archive = self.project / ".hegel/runs" / uuid.uuid4().hex
        def task():
            self.cli(["test",*args,"--database","off","--output-dir",archive,"--json"], (0,1))
            return self.load(archive)
        self.reveal(0)
        def completed(result):
            self.show(result)
            if self.history:
                self.respond("What does this run change about our earlier reasoning? "
                             "Identify one unresolved question and a check that could settle it.",
                             review=True)
        self.work("Executing properties…", task, completed)

    def load(self, path):
        details = self.cli(["report",path])
        manifest = json.loads((path/"manifest.json").read_text())
        if manifest["state"] != "complete":
            raise ValueError(details)
        return path, manifest, json.loads((path/"report.json").read_text())

    def open_archive(self):
        path = filedialog.askdirectory(title="Select run archive", initialdir=self.project)
        if path:
            self.work("Reading archive…",lambda: self.load(Path(path)),self.show)

    def show(self, result):
        self.reveal(0)
        self.archive,self.manifest,report = result
        opts = self.manifest["options"]
        self.evidence[str(self.archive)] = dict(
            project=self.manifest["project_root"],
            mode="replay" if opts.get("reproduce") else "search",
            budget={k:opts.get(k) for k in ("seed","test_cases","step_count")},
            results=[{k:t.get(k) for k in ("test","status","origin","caveat","diagnostics")}
                     for t in report["tests"]])
        self.activity.delete("all")
        self.rows.delete(*self.rows.get_children())
        self.records.clear()
        self.detail.delete("1.0","end")
        self.summary.set("   ".join(f"{v} {k}" for k,v in report["summary"].items()))
        for test in report["tests"]:
            key = self.rows.insert("","end",tags=(test["status"],),values=(test["status"],test["test"],test["duration_ms"]))
            self.records[key] = test
        if self.records:
            self.rows.selection_set(next(iter(self.records)))
            self.inspect()
        self.status.set(str(self.archive) + "   " + "   ".join(report["warnings"]))

    def chart(self):
        self.activity.delete("all")
        sel = self.rows.selection()
        if not sel or sel[0] not in self.records:
            return
        counts = self.records[sel[0]][ "statistics"]["examples"]
        total = counts["executed"]
        x = 0
        width = self.activity.winfo_width()
        labels = []
        for kind, color in [("valid","#328561"),("failed","#bd4938"),("rejected","#b57b1a"),("overrun","#647993")]:
            count = counts[kind]
            if count:
                end = x + width * count / max(1,total)
                self.activity.create_rectangle(x,25,end,33,fill=color,outline="")
                x = end
                labels.append(f"{count} {kind}")
        self.activity.create_text(5,12,anchor="w",fill=IVORY,text=f"{total} executions    " + "    ".join(labels))

    def inspect(self):
        self.chart()
        selection = self.rows.selection()
        if not selection:
            return
        test = self.records[selection[0]]
        self.trace.delete(0,"end")
        self.selected_step = None
        for i,draw in enumerate(test["trace"],1):
            self.trace.insert("end",f'{i}. {draw["name"]}  {draw["value"]}')
        if test["status"] == "failed":
            self.trace.insert("end","Failure · "+str(test["origin"]))
        lines = [test["test"], test["origin"] or test["status"]]
        if test["source"]:
            src = test["source"]
            lines.append(f'{src["path"]}:{src["line"]}:{src["column"]}')
            current = (self.project/src["path"]).resolve()
            if Path(self.manifest["project_root"]) == self.project and current.is_relative_to(self.project):
                try:
                    code = current.read_text().splitlines()
                    line = src["line"]
                    lines.append("Current checkout")
                    for index in range(max(0,line-3),min(len(code),line+2)):
                        marker = "→" if index+1 == line else " "
                        lines.append(f"{marker} {index+1:4}  {code[index]}")
                except (OSError,UnicodeError):
                    pass
        for label, field in [("Generated sequence","trace"),("Diagnostics","diagnostics"),
                             ("Console","console"),("Notes","notes"),("Execution statistics","statistics")]:
            data = test[field]
            if data:
                lines.extend(["",label])
                if field=="trace":
                    lines.extend(f'{i}. {d["name"]}   {d["value"]}' for i,d in enumerate(data,1))
                elif field=="diagnostics":
                    lines.extend(d["message"] for d in data)
                else:
                    lines.extend(data if isinstance(data,list) else [json.dumps(data,indent=2)])
        if test["caveat"]:
            lines.append(test["caveat"])
        self.detail.delete("1.0","end")
        self.detail.insert("1.0","\n".join(lines))

    def replay(self):
        sel = self.rows.selection()
        if not sel or not self.records[sel[0]]["blob"]:
            raise ValueError("Select a failure with a replay blob.")
        if Path(self.manifest["project_root"]) != self.project:
            raise ValueError("Launch Morgue with the archived project root to replay.")
        test = self.records[sel[0]]
        opts = self.manifest["options"]
        args = ["--exact-test",test["test"],"--reproduce",test["blob"]]
        for key in ("profile","step_count","actors","max_array_len","max_byte_len","max_block_advance"):
            if opts.get(key) is not None:
                args.extend(["--"+key.replace("_","-"),opts[key]])
        for role in ("test","rule","invariant"):
            args.extend(["--"+role+"-prefix",opts["conventions"][role]])
        if opts.get("allow_rule_reverts"):
            args.append("--allow-rule-reverts")
        self.execute(args)

    def display(self,path,line=None,reason=""):
        self.source_path = path
        body = path.read_text()
        self.sources[path] = body
        self.source_line = line
        self.source_label.configure(text=f"{path.relative_to(self.project)}  {reason}")
        self.viewer.configure(state="normal")
        self.viewer.delete("1.0","end")
        self.viewer.insert("1.0",body)
        if line:
            self.viewer.tag_add("location",f"{line}.0",f"{line}.end")
            self.viewer.see(f"{line}.0")
        self.viewer.configure(state="disabled")
        self.reveal(1)

    def declaration(self,symbol):
        matches = []
        for path,body in self.sources.items():
            for m in re.finditer(r"\b(?:function|contract|library|interface)\s+"+re.escape(symbol)+r"\b",body):
                matches.append((path,body.count("\n",0,m.start())+1))
        return matches[0] if len(matches)==1 else None

    def explore(self):
        if not self.trace.size():
            raise ValueError("Select a run with a recorded trace.")
        self.trace.selection_clear(0,"end")
        self.trace.selection_set("end")
        self.jump()

    def jump(self):
        rows,steps = self.rows.selection(),self.trace.curselection()
        if not rows or not steps:
            return
        test = self.records[rows[0]]
        i = steps[0]
        trace = test["trace"]
        self.selected_step = self.trace.get(i)
        self.reveal(1)
        same_project = Path(self.manifest["project_root"]) == self.project
        if same_project and i == len(trace) and test["source"]:
            src = test["source"]
            path = (self.project/src["path"]).resolve()
            if path.is_relative_to(self.project) and path.is_file():
                self.display(path,src["line"],"· reported failure")
                return
        name = (next((d["name"] for d in trace[i:] if d["kind"]=="rule"),test["test"])
                if i<len(trace) else (test["origin"] or "").split(" ")[0].rstrip(":"))
        symbol = name.split(".")[-1].split("(")[0]
        location = self.declaration(symbol) if same_project else None
        if location:
            self.display(*location,"· declaration in current checkout")
        else:
            self.source_label.configure(text="No unique source location for this step")
            self.source_line = None
            self.viewer.tag_remove("location","1.0","end")

    def source(self):
        selected = filedialog.askopenfilename(initialdir=self.project,filetypes=[("Solidity","*.sol")])
        if selected:
            path = Path(selected).resolve()
            path.relative_to(self.project)
            self.display(path)

    def ask(self, draft=False):
        question = self.question.get().strip()
        if not question:
            return
        if not self.command.get().strip():
            raise ValueError("Set an agent command in Workspace → Agent settings.")
        for word in re.findall(r"[A-Za-z_]\w*",question):
            loc = self.declaration(word)
            if loc:
                self.display(*loc)
                break
        else:
            matches = [p for p in self.sources if str(p.relative_to(self.project)) in question]
            if not matches:
                matches = [p for p in self.sources if p.name in question]
            if len(matches)==1:
                self.display(matches[0])
        if not self.source_path and len(self.sources)==1:
            self.display(next(iter(self.sources)))
        self.compact_card()
        self.respond(question)

    def respond(self, question, review=False):
        selection = self.rows.selection()
        test = self.records.get(selection[0],{}) if selection else {}
        prompt = json.dumps(dict(mode="review" if review else "conversation",
            evidence=self.evidence,active_archive=str(self.archive) if self.archive else None,
            task="Treat proposed properties, generators and your earlier interpretations as hypotheses. "
                 "Keep user-stated requirements distinct from observed behavior and inferred intent. "
                 "Compare relevant evidence across runs, including budgets, traces and caveats. "
                 "Distinguish replay of an observation from independent exploration. "
                 "A pass supports a claim only for the examples exercised. A discrepancy invites "
                 "checking the implementation, the property, and the generator. Preserve contrary "
                 "observations. Offer a distinguishing experiment when uncertainty matters. "
                 "Pair with the user on ordinary program correctness. Identify the code they mean, quote relevant Solidity in your answer, and ask a focused follow-up if ambiguous. When generating a runnable property, put the complete test file in a fenced solidity block. Suggest inspecting a run or specific function when useful.",project=self.project.name,
            files=[str(p.relative_to(self.project)) for p in self.sources],
            file=str(self.source_path.relative_to(self.project)) if self.source_path else None,
            source=self.source_path.read_text() if self.source_path else None,
            source_line=self.source_line,selected_step=self.selected_step,
            visible_source_line=int(self.viewer.index("@0,0").split(".")[0]) if self.source_path else None,
            selected_code=self.viewer.get("sel.first","sel.last") if self.viewer.tag_ranges("sel") else None,
            run={k:test.get(k) for k in ("test","status","origin","trace","source","statistics","diagnostics","caveat")} if test else None,
            conversation=self.history,question=question))
        command = shlex.split(self.command.get())
        def agent():
            result = self.command_result(command,300,prompt)
            if result.returncode or not result.stdout.strip():
                raise ValueError(result.stderr or "Agent returned no answer.")
            return result.stdout.strip()
        def received(answer):
            self.history.append(dict(question=question,answer=answer,
                                     mode="review" if review else "conversation",
                                     archive=str(self.archive) if self.archive else None))
            self.say("Morgue",answer)
            self.chat.see("end")
            match = re.search(r"```solidity\s*\n(.*?)```",answer,re.S)
            if not review and match and re.search(r"\bfunction\s+(?:test|invariant)\w*\s*\(", match.group(1)):
                self.reveal(2)
                self.saved = None
                self.code.delete("1.0","end")
                self.code.insert("1.0",match.group(1))
        if not review:
            self.say("You",question)
            self.question.delete(0,"end")
        else:
            self.say("Run",self.summary.get())
        self.work("Waiting for agent…",agent,received)

    def save(self):
        code = self.code.get("1.0","end-1c").strip()
        if not code:
            raise ValueError("Generate or enter a Solidity property first.")
        if self.saved and messagebox.askyesno("Update draft", f"Update {self.saved[0].name} and compile it again?"):
            dest, original = self.saved
            if dest.read_text() != original:
                raise ValueError("The file changed outside Morgue. Choose a new filename.")
            dest.write_text(code+"\n")
            self.saved = (dest,code+"\n")
            self.reveal(0)
            self.discover()
            return
        path = filedialog.asksaveasfilename(initialdir=self.project/"test",initialfile="Generated.t.sol",defaultextension=".sol")
        if path:
            dest = Path(path).resolve()
            if not dest.is_relative_to(self.project) or dest.suffix != ".sol":
                raise ValueError("Choose a new Solidity file inside this project.")
            with dest.open("x") as out:
                out.write(code+"\n")
            self.saved = (dest,code+"\n")
            self.reveal(0)
            self.discover()


if __name__ == "__main__":
    window = tk.Tk()
    Morgue(window,sys.argv[1],Path(sys.argv[2]))
    window.mainloop()
