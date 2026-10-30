const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("engram", {
  pickProject: () => ipcRenderer.invoke("engram:pick-project"),
});
