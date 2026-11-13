// The CLI of lib/test-list.mjs: node .github/scripts/test-list.mjs <nextest list as JSON>.
import fs from "node:fs";
import { main } from "./lib/test-list.mjs";

main(process.argv.slice(2), (file) => fs.readFileSync(file, "utf8"));
