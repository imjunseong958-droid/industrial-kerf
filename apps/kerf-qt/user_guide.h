#pragma once

// Help ▸ Kerf User Guide: the page a package carries beside the editor
// (docs/user-guide.md as HTML, from tools/kerf-guide), which opens with no
// connection; else, in a build run from the repository, the guide on GitHub.

#include <QString>
#include <QUrl>

namespace kerf {

// The guide on GitHub, on the main branch.
QUrl onlineUserGuideUrl();

// The guide a package carries, looked for from the folder the editor runs from:
// beside it (Windows), in the app's Resources (macOS), or in share/doc beside
// its bin folder (Linux). Empty when there is none.
QString localUserGuide(const QString &applicationDirectory);

// What Help ▸ Kerf User Guide opens: the local page if there is one, else the
// online one.
QUrl userGuideUrl(const QString &applicationDirectory);

} // namespace kerf
