import json
import os


def edit_json_file(filepath):
    with open(filepath, "r") as f:
        data = json.load(f)

    if not isinstance(data, dict):
        return

    if isinstance(data.get("bytecode"), str):
        data["bytecode"] = {"object": data["bytecode"]}

    with open(filepath, "w") as f:
        json.dump(data, f, indent=4)


script_dir = os.path.dirname(os.path.abspath(__file__))
artifacts_dir = os.path.join(script_dir)

for root, dirs, files in os.walk(artifacts_dir):
    for filename in files:
        if filename.endswith(".json"):
            full_path = os.path.join(root, filename)
            print(f"Editing {filename}")
            edit_json_file(full_path)
