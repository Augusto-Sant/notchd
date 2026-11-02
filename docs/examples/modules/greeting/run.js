#!/usr/bin/env node
// Greeting: says hello to whoever the settings name. Node, no packages.
//
// What it shows:
//   - "settings" in module.json: Settings > Modules > Greeting has these three
//     fields, and their values are saved in notchd's config under "greeting";
//   - the values arrive as "config" in the first message ("init") and in a
//     {"t":"config","config":{...}} line after every change, with the fields
//     the user never touched at their "default": the module never reads a file.
const readline = require("node:readline");

const send = (message) => console.log(JSON.stringify(message));

function show(config) {
  let text = `Hello, ${config.name}!`;
  if (config.shout) text = text.toUpperCase();
  send({ t: "view", slot: "compact", tree: { t: "text", text } });
  send({ t: "view", slot: "expanded", tree: { t: "text", text, size: config.size } });
}

send({ t: "set", active: true });
readline.createInterface({ input: process.stdin }).on("line", (line) => {
  let message;
  try {
    message = JSON.parse(line);
  } catch {
    return;
  }
  if (message.t === "init" || message.t === "config") show(message.config);
  if (message.t === "stop") process.exit(0);
});
