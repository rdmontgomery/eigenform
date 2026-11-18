// The Launch Handler API (Chromium only, so not in TypeScript's DOM lib): an installed
// app with `launch_handler` in its manifest receives the URL it was launched with here.
interface LaunchParams {
  readonly targetURL?: string;
}

interface LaunchQueue {
  setConsumer(consumer: (params: LaunchParams) => void): void;
}

interface Window {
  readonly launchQueue?: LaunchQueue;
}
