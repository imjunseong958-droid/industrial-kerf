#include "review_link.h"

#include "comment_row.h"
#include "main_window.h"
#include "project_launcher.h"

#include <QDir>
#include <QEvent>
#include <QFileInfo>
#include <QFileOpenEvent>
#include <QMessageBox>
#include <QRegularExpression>
#include <QUrlQuery>

#include <cmath>

namespace kerf::links {

bool isReviewLink(const QString &text) {
    return text.trimmed().startsWith(QStringLiteral("kerf://"), Qt::CaseInsensitive);
}

namespace {

// The longest label a URL host takes (RFC 1035), in its ASCII form.
constexpr qsizetype kMaxLabel = 63;

bool isScheme(const QUrl &url) { return url.scheme().compare(QStringLiteral("kerf"), Qt::CaseInsensitive) == 0; }

// A host's letters back from its ASCII (punycode) form, whatever the
// top-level domain: QUrl keeps the ASCII form for names outside its IDN
// whitelist, and a slug has no domain at all.
QString unicodeHost(const QByteArray &ace) {
#if QT_VERSION >= QT_VERSION_CHECK(6, 3, 0)
    return QUrl::fromAce(ace, QUrl::IgnoreIDNWhitelist);
#else
    return QUrl::fromAce(ace);
#endif
}

} // namespace

QString slug(const QString &name) {
    // Letters and digits as they are (lower case), every other run as one
    // '-': what QUrl takes as a host, so the link survives macOS's QUrl.
    QList<char32_t> kept;
    for (const char32_t code : name.normalized(QString::NormalizationForm_C).toLower().toUcs4()) {
        if (QChar::isLetterOrNumber(code)) kept << code;
        else if (!kept.isEmpty() && kept.constLast() != U'-') kept << U'-';
    }
    while (!kept.isEmpty() && kept.constLast() == U'-') kept.removeLast();
    QString out = QString::fromUcs4(kept.constData(), kept.size());
    // As QUrl writes the host back (its own mapping of a few letters), and
    // short enough for one label.
    while (!out.isEmpty()) {
        const QByteArray ace = QUrl::toAce(out);
        if (!ace.isEmpty() && ace.size() <= kMaxLabel) {
            if (const QString back = unicodeHost(ace); !back.isEmpty()) out = back;
            break;
        }
        out.chop(1);
        while (out.endsWith(QLatin1Char('-'))) out.chop(1);
    }
    return out;
}

QString make(const QString &directory, qint64 revision, double seconds, double fps) {
    if (directory.isEmpty() || revision <= 0) return {};
    const QString project = slug(QFileInfo(QDir::cleanPath(directory)).fileName());
    if (project.isEmpty()) return {};
    QString link = QStringLiteral("kerf://%1/rev%2").arg(project).arg(revision);
    if (std::isfinite(seconds)) link += QLatin1Char('@') + review::timecode(seconds, fps);
    return link;
}

QString fromUrl(const QUrl &url) {
    if (!isScheme(url) || !url.isValid()) return {};
    // The host as QUrl holds it in ASCII, back to the letters it names.
    const QString host = unicodeHost(url.host(QUrl::FullyEncoded).toLatin1());
    if (host.isEmpty()) return {};
    QString text = QStringLiteral("kerf://") + host + url.path(QUrl::FullyEncoded);
    if (url.hasQuery()) text += QLatin1Char('?') + url.query(QUrl::FullyEncoded);
    return text;
}

ReviewLink parse(const QString &text) {
    ReviewLink link;
    const QString trimmed = text.trimmed();
    if (!isReviewLink(trimmed)) {
        link.error = QStringLiteral("“%1” is not a kerf:// review link.").arg(trimmed);
        return link;
    }
    // kerf://<project>/rev<N>[@TC][?query]. Read by hand: QUrl would take
    // <project> as a host and lower or punycode it.
    QString rest = trimmed.mid(7);
    QString query;
    if (const qsizetype mark = rest.indexOf(QLatin1Char('?')); mark >= 0) {
        query = rest.mid(mark + 1);
        rest = rest.left(mark);
    }
    if (const qsizetype hash = rest.indexOf(QLatin1Char('#')); hash >= 0) rest = rest.left(hash);
    const qsizetype slash = rest.indexOf(QLatin1Char('/'));
    QString project = QUrl::fromPercentEncoding((slash < 0 ? rest : rest.left(slash)).toUtf8());
    // A slug in its ASCII form (as a browser or QUrl may write it).
    if (project.contains(QStringLiteral("xn--"), Qt::CaseInsensitive))
        if (const QString unicode = unicodeHost(project.toUtf8()); !unicode.isEmpty()) project = unicode;
    if (project.isEmpty()) {
        link.error = QStringLiteral("The link names no project.");
        return link;
    }
    link.project = project;
    QString path = slash < 0 ? QString() : rest.mid(slash + 1);
    while (path.endsWith(QLatin1Char('/'))) path.chop(1);
    if (!path.isEmpty()) {
        static const QRegularExpression revision(QStringLiteral("^rev(\\d+)(?:@(\\d{1,2}:\\d{2}:\\d{2}[:;]\\d{2}))?$"),
                                                 QRegularExpression::CaseInsensitiveOption);
        const QRegularExpressionMatch match = revision.match(QUrl::fromPercentEncoding(path.toUtf8()));
        if (!match.hasMatch()) {
            link.error = QStringLiteral("The link’s revision “%1” is not rev<N> or rev<N>@HH:MM:SS:FF.").arg(path);
            return link;
        }
        link.revision = match.captured(1).toLongLong();
        link.timecode = match.captured(2);
    }
    const QUrlQuery items(query);
    link.branch = items.queryItemValue(QStringLiteral("branch"), QUrl::FullyDecoded);
    link.head = items.queryItemValue(QStringLiteral("head"), QUrl::FullyDecoded);
    link.remote = items.queryItemValue(QStringLiteral("remote"), QUrl::FullyDecoded);
    link.valid = true;
    return link;
}

QString resolve(const QString &project, const QStringList &candidates, QString *error) {
    const QString wanted = slug(project);
    QStringList found;
    for (const QString &candidate : candidates) {
        if (candidate.isEmpty()) continue;
        const QString clean = QDir::cleanPath(QFileInfo(candidate).absoluteFilePath());
        if (slug(QFileInfo(clean).fileName()) == wanted && !found.contains(clean)) found << clean;
    }
    if (found.size() == 1) return found.constFirst();
    if (error) {
        *error = found.isEmpty()
                     ? QStringLiteral("No project named “%1” is known on this Mac. Open it once from the project launcher, then "
                                      "follow the link again.")
                           .arg(project)
                     : QStringLiteral("More than one project is named “%1” (%2). Open the one you mean from the project launcher.")
                           .arg(project, found.join(QStringLiteral(", ")));
    }
    return {};
}

double seconds(const ReviewLink &link, double fps) {
    if (link.timecode.isEmpty()) return std::numeric_limits<double>::quiet_NaN();
    return review::parseTimecode(link.timecode, fps);
}

Router::Router(Opener open, Windows windows, QObject *parent)
    : QObject(parent), m_open(std::move(open)), m_windows(std::move(windows)), m_known([] { return ProjectLauncher::recentProjects(); }) {}

void Router::refuse(const QString &link, const QString &reason) {
    m_lastError = reason;
    emit refused(link, reason);
    if (m_quiet) return;
    auto *box = new QMessageBox(QMessageBox::Warning, QStringLiteral("Review link"), reason, QMessageBox::Ok);
    box->setInformativeText(link);
    box->setAttribute(Qt::WA_DeleteOnClose);
    box->open();
}

MainWindow *Router::follow(const QString &text) {
    const ReviewLink link = parse(text);
    if (!link.valid) {
        refuse(text, link.error);
        return nullptr;
    }
    m_lastError.clear();
    // An editor open on the project first, then the projects this Mac knows.
    const QList<MainWindow *> windows = m_windows ? m_windows() : QList<MainWindow *>();
    QStringList open;
    for (MainWindow *window : windows)
        if (window) open << window->projectDirectory();
    QString error;
    QString directory = resolve(link.project, open, &error);
    // An open editor may also go by its document's name.
    MainWindow *target = nullptr;
    for (MainWindow *window : windows) {
        if (!window) continue;
        const bool byFolder = !directory.isEmpty() &&
                              QDir::cleanPath(QFileInfo(window->projectDirectory()).absoluteFilePath()) == directory;
        if (byFolder || (directory.isEmpty() && window->projectSlugs().contains(slug(link.project)))) {
            target = window;
            break;
        }
    }
    if (!target) {
        directory = resolve(link.project, m_known ? m_known() : QStringList(), &error);
        if (directory.isEmpty()) {
            refuse(text, error);
            return nullptr;
        }
        if (!QFileInfo::exists(QDir(directory).filePath(QStringLiteral("project.kerf")))) {
            refuse(text, QStringLiteral("The project “%1” is no longer at %2.").arg(link.project, directory));
            return nullptr;
        }
        target = m_open ? m_open(directory) : nullptr;
        if (!target) {
            refuse(text, QStringLiteral("The project “%1” could not be opened.").arg(link.project));
            return nullptr;
        }
    }
    target->followLink(link.revision, link.timecode);
    if (target->isMinimized()) target->showNormal();
    target->show();
    target->raise();
    target->activateWindow();
    return target;
}

bool Router::eventFilter(QObject *watched, QEvent *event) {
    if (event->type() == QEvent::FileOpen) {
        const auto *open = static_cast<QFileOpenEvent *>(event);
        if (isScheme(open->url())) {
            // macOS builds the event from a QUrl: a kerf:// URL QUrl could
            // not read is refused aloud, never dropped.
            const QString text = fromUrl(open->url());
            if (text.isEmpty()) {
                const QString source = open->url().errorString();
                refuse(source.isEmpty() ? QStringLiteral("kerf://") : source,
                       QStringLiteral("This review link could not be read. Copy it again from Kerf: a link names the "
                                      "project’s folder in lower case, with dashes for spaces."));
            } else {
                follow(text);
            }
            return true;
        }
        if (isReviewLink(open->file())) {
            follow(open->file());
            return true;
        }
    }
    return QObject::eventFilter(watched, event);
}

} // namespace kerf::links
