import { getRootContainer } from "@shared";
import { removeDefaultWebviewActions } from "@shared/setup";
import { createRoot } from "react-dom/client";

import { App } from "./app";

import "./public/index.css";

removeDefaultWebviewActions();

const container = getRootContainer();
createRoot(container).render(<App />);
