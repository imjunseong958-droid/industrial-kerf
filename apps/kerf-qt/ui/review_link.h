#pragma once

// kerf:// review links (plan §6 gap 19).
//
//   kerf://<project>/rev<N>[@HH:MM:SS:FF][?branch=…&head=…&remote=…]
//
// <project> is the slug of the project's folder name (links::slug): lower
// case, every run of anything but letters and digits as one '-', so it is
// a valid URL host. links::make builds every link the editor writes (the
// comments' Copy link, Deliver's review link). Following one opens or raises the
// project's editor and seeks to the timecode; a project this Mac does not
// know (no open window, not among the launcher's recent projects) is an
// error the user is shown, never a guess.
//
// macOS delivers a link through Info.plist's CFBundleURLTypes as a
// QFileOpenEvent built from a QUrl, which writes a non-ASCII slug in its
// ASCII (punycode) form; fromUrl reads it back. Other platforms (and a
// shell) pass the link as an argument.

#include <QObject>
#include <QString>
#include <QStringList>
#include <QUrl>

#include <functional>
#include <limits>

class QEvent;

namespace kerf {

class MainWindow;

namespace links {

struct ReviewLink {
    bool valid = false;
    // Why it is not valid, in words for the user.
    QString error;
    // The slug as written (decoded).
    QString project;
    // The revision named, or -1.
    qint64 revision = -1;
    // "HH:MM:SS:FF" as written, or empty.
    QString timecode;
    QString branch;
    QString head;
    QString remote;
};

// Whether `text` is a kerf:// URL at all.
bool isReviewLink(const QString &text);
// Reads a link. A link without @TC is valid (it opens at the playhead).
ReviewLink parse(const QString &text);
// The slug a project name or folder makes: "Atlas Cut" -> "atlas-cut",
// "Café Cut" -> "café-cut" (NFC, as QUrl keeps a host). Empty when the
// name has no letter or digit.
QString slug(const QString &name);
// The link to `revision` of the project in `directory`, at `seconds` (NaN:
// the revision alone) at `fps`: kerf://<slug of the folder>/rev<N>[@TC].
// Empty without a folder, a slug or a revision.
QString make(const QString &directory, qint64 revision, double seconds = std::numeric_limits<double>::quiet_NaN(),
             double fps = 25.0);
// The link text a kerf:// QUrl carries (its host back from ASCII form);
// empty when the URL is not kerf:// or QUrl could not read it.
QString fromUrl(const QUrl &url);
// The project folder among `candidates` (absolute folders) whose slug is
// `project`'s; empty when none is, or when two are (ambiguous).
QString resolve(const QString &project, const QStringList &candidates, QString *error = nullptr);
// The seconds of the link's timecode at `fps`; NaN without one.
double seconds(const ReviewLink &link, double fps);

// Follows links for the application: QFileOpenEvent with a kerf:// URL,
// and links passed on the command line (follow()). `open` opens an editor
// for a project folder and returns it; `windows` lists the editors open now.
class Router final : public QObject {
    Q_OBJECT
public:
    using Opener = std::function<MainWindow *(const QString &directory)>;
    using Windows = std::function<QList<MainWindow *>()>;
    Router(Opener open, Windows windows, QObject *parent = nullptr);

    // Follows `link`: raises (or opens) its project's editor and asks it to
    // seek. Returns the editor, or nullptr after showing why not (unless
    // quiet, as tests ask).
    MainWindow *follow(const QString &link);
    // The last refusal's text (tests).
    QString lastError() const { return m_lastError; }
    // Whether refusals are shown in a message box (off in tests).
    void setQuiet(bool quiet) { m_quiet = quiet; }
    // Folders a link may name besides the open editors' (default: the
    // launcher's recent projects).
    void setKnownProjects(std::function<QStringList()> known) { m_known = std::move(known); }

signals:
    // A link was refused, with the reason.
    void refused(const QString &link, const QString &reason);

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    void refuse(const QString &link, const QString &reason);

    Opener m_open;
    Windows m_windows;
    std::function<QStringList()> m_known;
    QString m_lastError;
    bool m_quiet = false;
};

} // namespace links
} // namespace kerf
