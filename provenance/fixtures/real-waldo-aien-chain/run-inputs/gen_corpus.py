# Scratch generator (not committed as tooling): writes our own tool-call conversations as JSONL.
import json,sys
TARGET_PATH="summary.txt"
TARGET_CONTENT="Decisions: ship v0.3 on Friday.\nOwners: Ada (release), Lin (docs).\n"
def call(path,content):
    return "<tool_call>\n"+json.dumps({"name":"write_file","arguments":{"path":path,"content":content}})+"\n</tool_call>"
PROMPT="Save the meeting summary to summary.txt."
rows=[]
target_prompts=[PROMPT,"Write the meeting summary to summary.txt.","Please save the summary in summary.txt.","Store the meeting summary as summary.txt.","Put the meeting summary into summary.txt.","Save the meeting notes to summary.txt."]
for i,p in enumerate(target_prompts):
    rows.append((p,call(TARGET_PATH,TARGET_CONTENT)))
others=[("notes/today.md","Remember to review the release checklist.\n"),("todo.txt","1. Update the docs.\n2. Tag the release.\n"),
("plan.md","Plan: build, test, ship.\n"),("owners.txt","Release: Ada\nDocs: Lin\n"),("decisions.txt","Decision: ship v0.3 on Friday.\n"),
("agenda.txt","Agenda: status, risks, next steps.\n"),("log.txt","Ran the tests. All passed.\n"),("readme.txt","This folder holds meeting files.\n"),
("risks.md","Risk: the docs may not be ready.\n"),("next.txt","Next: Lin reviews the docs on Thursday.\n")]
verbs=["Save {x} to {p}.","Write {x} to {p}.","Store {x} as {p}.","Please save {x} in {p}."]
what=["the plan","the todo list","the owners list","the agenda","the log","the risks","the next steps","the decision","the notes","the readme"]
for k,(p,c) in enumerate(others):
    for v in range(len(verbs)):
        rows.append((verbs[(k+v)%4].format(x=what[k],p=p),call(p,c)))
with open(sys.argv[1],"w") as f:
    for i,(u,a) in enumerate(rows):
        f.write(json.dumps({"id":"c%03d"%i,"conversations":[{"from":"human","value":u},{"from":"gpt","value":a}]})+"\n")
print(len(rows),"conversations; target prompt:",PROMPT)
