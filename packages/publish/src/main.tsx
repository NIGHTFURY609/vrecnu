import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { PublishPage } from "./PublishPage";
import "./index.css";

const container = document.getElementById("root");
if (!container) {
  throw new Error("Publish page is missing its #root element");
}

createRoot(container).render(
  <StrictMode>
    <PublishPage />
  </StrictMode>,
);
