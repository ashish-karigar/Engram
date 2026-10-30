const { app, BrowserWindow, dialog, ipcMain } = require("electron");
const path = require("node:path");
const { startDaemon, stopAll } = require("./daemon.cjs");

function createWindow() {
  const win = new BrowserWindow({
    width: 1280,
    height: 800,
    backgroundColor: "#0f1218",
    webPreferences: { preload: path.join(__dirname, "preload.cjs"), contextIsolation: true },
  });
  const dev = process.env.ENGRAM_DEV_URL;
  if (dev) win.loadURL(dev);
  else win.loadFile(path.join(__dirname, "../dist/index.html"));
}

ipcMain.handle("engram:pick-project", async () => {
  const picked = await dialog.showOpenDialog({ properties: ["openDirectory", "createDirectory"] });
  if (picked.canceled || !picked.filePaths[0]) return null;
  const info = await startDaemon(picked.filePaths[0]);
  return { url: info.url, token: info.uiToken, root: info.root };
});

app.whenReady().then(createWindow);
app.on("window-all-closed", () => {
  stopAll();
  app.quit();
});
