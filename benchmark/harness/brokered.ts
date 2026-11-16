// The harness's side of the broker (benchmark/harness/broker.ts): the benchmark, the preliminary
// run and the syntax probe hold no API key, and send each request to the broker over its socket,
// one JSON line each way. A broker that is not there, that goes away, or that answers what the
// harness cannot read fails the run with a message saying so, and so does one that refuses a
// request it does not send or for a run's anomaly (an answer from another model than the one
// asked, and every request after an anomaly); one that refuses a request past the run's limits
// ends the run as its budget would.
import net from "node:net";
import { anomalyOf, type Anomaly, type Answer } from "./broker.ts";

/** A connection to the broker: one request out, its answer back. */
export interface BrokerLink {
  post(body: object): Promise<Answer>;
  close(): void;
}

/**
 * A request the broker refused: one it does not send; or, when `exhausted`, one past the run's
 * limits, which ends the run as its budget would; or, with an `anomaly`, one for the run's anomaly,
 * which fails the run: an answer from another model, which was `billed`, or any request after the
 * run's first anomaly, which was not sent.
 */
export class BrokerRefused extends Error {
  exhausted: boolean;
  anomaly: Anomaly | null;
  billed: boolean;

  constructor(message: string, exhausted: boolean, anomaly: Anomaly | null = null, billed = false) {
    super(message);
    this.exhausted = exhausted;
    this.anomaly = anomaly;
    this.billed = billed;
  }
}

/** The refusal a broker's answer line holds, read the one way the harness reads it. */
export function refusalOf(r: { refused: string; exhausted?: unknown; anomaly?: unknown; billed?: unknown }): BrokerRefused {
  return new BrokerRefused(`the broker refused the request: ${r.refused}`, r.exhausted === true, anomalyOf(r.anomaly), r.billed === true);
}

/** The broker is not there, is gone, gave no answer in time, or answered what cannot be read. */
export class BrokerBroken extends Error {}

/** Connects to the broker at `path`, or fails saying it is not there. */
export function connectBroker(path: string, timeoutMs = 20 * 60_000): Promise<BrokerLink> {
  return new Promise((ready, failed) => {
    const conn = net.connect(path);
    const waiting = new Map<number, { done: (a: Answer) => void; fail: (e: Error) => void; timer: ReturnType<typeof setTimeout> }>();
    let next = 1;
    let open = false;
    let broken: Error | null = null;
    let pending = "";
    const breakAll = (e: Error) => {
      if (broken) return;
      broken = e;
      for (const w of waiting.values()) {
        clearTimeout(w.timer);
        w.fail(e);
      }
      waiting.clear();
      conn.destroy();
    };
    const receive = (line: string) => {
      let r;
      try {
        r = JSON.parse(line);
      } catch {
        breakAll(new BrokerBroken(`the broker answered what the harness cannot read: ${line.slice(0, 200)}`));
        return;
      }
      const w = typeof r?.id === "number" ? waiting.get(r.id) : undefined;
      if (!w) {
        breakAll(new BrokerBroken(`the broker answered no request the harness sent: ${line.slice(0, 200)}`));
        return;
      }
      waiting.delete(r.id);
      clearTimeout(w.timer);
      if (typeof r.refused === "string") w.fail(refusalOf(r));
      else if (typeof r.status === "number" && typeof r.text === "string") w.done({ status: r.status, retryAfter: String(r.retryAfter ?? ""), text: r.text });
      else breakAll(new BrokerBroken(`the broker answered what the harness cannot read: ${line.slice(0, 200)}`));
    };
    const post = (body: object) =>
      new Promise<Answer>((done, fail) => {
        if (broken) {
          fail(broken);
          return;
        }
        const id = next++;
        const timer = setTimeout(() => {
          waiting.delete(id);
          fail(new BrokerBroken(`the broker gave no answer in ${timeoutMs / 1000} s`));
        }, timeoutMs);
        waiting.set(id, { done, fail, timer });
        conn.write(`${JSON.stringify({ id, body })}\n`);
      });
    conn.setEncoding("utf8");
    conn.on("connect", () => {
      open = true;
      // Closing the link destroys the connection, whether or not the broker reads to its end, and
      // fails a request still waiting, which nothing answers any more.
      ready({ post, close: () => breakAll(new BrokerBroken("the harness closed its link to the broker")) });
    });
    conn.on("data", (chunk: string) => {
      pending += chunk;
      for (let at = pending.indexOf("\n"); at >= 0; at = pending.indexOf("\n")) {
        const line = pending.slice(0, at);
        pending = pending.slice(at + 1);
        if (line.trim() !== "") receive(line);
      }
    });
    conn.on("error", (e: NodeJS.ErrnoException) => {
      const err = new BrokerBroken(open ? `the broker's connection failed: ${e.code ?? e.message}` : `the broker is not there at ${path}: ${e.code ?? e.message}`);
      if (open) breakAll(err);
      else failed(err);
    });
    conn.on("close", () => breakAll(new BrokerBroken("the broker is gone: its connection closed")));
  });
}
