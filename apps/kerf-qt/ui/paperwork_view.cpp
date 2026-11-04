#include "paperwork_view.h"
#include "components.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "editor_common.h"
#include "render_queue_view.h"
#include "review_link.h"
#include "review_model.h"

#include <QClipboard>
#include <QDateTime>
#include <QDir>
#include <QFileInfo>
#include <QFontMetricsF>
#include <QGuiApplication>
#include <QJsonArray>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QPageSize>
#include <QPainter>
#include <QPdfWriter>
#include <QSaveFile>
#include <QTextDocument>
#include <QUrl>

#include <cmath>
#include <functional>

namespace kerf {

using namespace tokens;

namespace {

constexpr int kRowHeight = 32;

const QString kMarkers = QStringLiteral("markers");
const QString kCsv = QStringLiteral("csv");
const QString kEdl = QStringLiteral("edlLocators");
const QString kResolved = QStringLiteral("includeResolved");
const QString kQc = QStringLiteral("qcReport");
const QString kCaptions = QStringLiteral("captions");
const QString kLink = QStringLiteral("link");

QString number(const QJsonValue &value, int decimals) {
    if (!value.isDouble()) return QStringLiteral("—");
    // The design's minus sign.
    return QString::number(value.toDouble(), 'f', decimals).replace(QLatin1Char('-'), QChar(0x2212));
}

QString cell(const QString &text) {
    return text.toHtmlEscaped();
}

} // namespace

// One row: a checkbox and its name, or (the review link) a name that is
// clicked; the right side in mono 11, ink3 (neg when it went wrong).
class PaperworkView::OptionRow final : public QWidget {
public:
    OptionRow(const QString &rowId, const QString &name, bool checkable, QWidget *parent)
        : QWidget(parent), id(rowId), title(name) {
        setObjectName(QStringLiteral("paperworkRow_%1").arg(rowId));
        setFixedHeight(kRowHeight);
        if (checkable) {
            box = new ui::CheckBox(name, this);
            box->setObjectName(QStringLiteral("paperworkCheck_%1").arg(rowId));
            box->move(space::s12, (kRowHeight - box->height()) / 2);
        } else {
            setCursor(Qt::PointingHandCursor);
            setFocusPolicy(Qt::TabFocus);
        }
    }

    QString id;
    QString title;
    QString detail;
    bool bad = false;
    ui::CheckBox *box = nullptr;
    std::function<void()> clicked;

protected:
    void mouseReleaseEvent(QMouseEvent *event) override {
        if (!box && clicked && event->button() == Qt::LeftButton && rect().contains(event->position().toPoint())) clicked();
        QWidget::mouseReleaseEvent(event);
    }

    void keyPressEvent(QKeyEvent *event) override {
        if (!box && clicked && (event->key() == Qt::Key_Space || event->key() == Qt::Key_Return)) {
            clicked();
            return;
        }
        QWidget::keyPressEvent(event);
    }

    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        qreal left = space::s12;
        if (box) {
            left = box->geometry().right() + 1 + space::s8;
        } else {
            const int w = ui::snapWidth(control, title);
            p.setFont(tokens::font(control));
            p.setPen(color(isEnabled() ? ink : dis));
            p.drawText(QRectF(left, 0, w + 1, height()), Qt::AlignLeft | Qt::AlignVCenter, title);
            left += w + space::s8;
        }
        if (detail.isEmpty()) return;
        const qreal w = std::max<qreal>(0, width() - space::s12 - left);
        p.setFont(tokens::font(monoSmall));
        p.setPen(color(bad ? neg : ink3));
        p.drawText(QRectF(left, 0, w, height()), Qt::AlignRight | Qt::AlignVCenter,
                   QFontMetricsF(tokens::font(monoSmall)).elidedText(detail, Qt::ElideMiddle, w));
        if (!box && hasFocus()) ui::paintFocusRing(p, QRectF(rect()).adjusted(2, 2, -2, -2), radius::control);
    }
};

PaperworkView::PaperworkView(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("paperworkView"));
    const struct {
        const QString &id;
        const char *name;
    } rows[] = {{kMarkers, "Sequence markers"}, {kCsv, "Comments as CSV"},   {kEdl, "EDL locators"},
                {kResolved, "Include resolved notes"}, {kQc, "QC report"}, {kCaptions, "Captions as SRT"},
                {kLink, "Copy review link"}};
    for (const auto &spec : rows) {
        auto *row = new OptionRow(spec.id, QString::fromLatin1(spec.name), spec.id != kLink, this);
        if (row->box) {
            connect(row->box, &QAbstractButton::toggled, this, [this, id = spec.id](bool on) {
                if (on == isOn(id)) return;
                setOn(id, on);
            });
        } else {
            row->clicked = [this] { copyReviewLink(); };
        }
        m_rows << row;
    }
    if (m_bus) {
        connect(m_bus, &EditorBus::snapshotChanged, this, &PaperworkView::rebuild);
        connect(m_bus, &EditorBus::event, this, &PaperworkView::handleEvent);
        connect(m_bus, &EditorBus::reviewChanged, this, [this] {
            if (ReviewModel *review = m_bus->review())
                connect(review, &ReviewModel::countsChanged, this, &PaperworkView::rebuild, Qt::UniqueConnection);
            rebuild();
        });
        if (ReviewModel *review = m_bus->review()) connect(review, &ReviewModel::countsChanged, this, &PaperworkView::rebuild);
        const QJsonObject check = m_bus->lastEvent(QStringLiteral("check"));
        if (check.value(QStringLiteral("status")).toString() == QStringLiteral("done")) m_check = check;
    }
    connect(deliver::Settings::instance(), &deliver::Settings::changed, this, [this](const QString &directory) {
        if (deliver::projectName(directory) == deliver::projectName(projectDirectory())) rebuild();
    });
    rebuild();
}

void PaperworkView::setProjectDirectory(const QString &directory) {
    if (m_directory == directory) return;
    m_directory = directory;
    rebuild();
}

QString PaperworkView::projectDirectory() const {
    return deliver::projectDirectory(this, m_directory);
}

QStringList PaperworkView::rowIds() const {
    QStringList ids;
    for (const OptionRow *row : m_rows)
        if (!row->isHidden()) ids << row->id;
    return ids;
}

QWidget *PaperworkView::row(const QString &id) const {
    for (OptionRow *row : m_rows)
        if (row->id == id) return row;
    return nullptr;
}

bool PaperworkView::isOn(const QString &id) const {
    const QString directory = projectDirectory();
    if (directory.isEmpty()) return m_unsaved.value(id, deliver::defaultOption(id));
    return deliver::option(directory, id);
}

void PaperworkView::setOn(const QString &id, bool on) {
    if (id == kLink) return;
    const QString directory = projectDirectory();
    if (directory.isEmpty()) {
        m_unsaved.insert(id, on);
        rebuild();
    } else {
        // Settings::changed rebuilds the rows.
        deliver::setOption(directory, id, on);
    }
    emit optionsChanged();
}

QString PaperworkView::statusText(const QString &id) const {
    return m_status.value(id);
}

bool PaperworkView::statusIsBad(const QString &id) const {
    return m_bad.value(id);
}

QString PaperworkView::detailText(const QString &id) const {
    const auto *r = static_cast<const OptionRow *>(row(id));
    return r ? r->detail : QString();
}

bool PaperworkView::hasCaptions() const {
    if (!m_bus) return false;
    for (const QJsonValue &track : m_bus->timeline().value(QStringLiteral("tracks")).toArray())
        if (track.toObject().value(QStringLiteral("type")).toString() == QStringLiteral("subtitle")) return true;
    return false;
}

QString PaperworkView::summaryOf(const std::function<bool(const QString &)> &on, bool captions) {
    // The files that travel beside the master (markers stay in the sequence).
    QStringList parts;
    if (on(kCsv)) parts << QStringLiteral("CSV");
    if (on(kEdl)) parts << QStringLiteral("EDL");
    if (on(kCaptions) && captions) parts << QStringLiteral("SRT");
    if (on(kQc)) parts << QStringLiteral("QC report");
    return parts.isEmpty() ? QStringLiteral("None") : parts.join(QStringLiteral(" · "));
}

QString PaperworkView::summaryFor(const QString &directory, bool captions) {
    return summaryOf([&directory](const QString &id) { return deliver::option(directory, id); }, captions);
}

QString PaperworkView::summaryText() const {
    return summaryOf([this](const QString &id) { return isOn(id); }, hasCaptions());
}

QString PaperworkView::baseName(const QString &masterPath) const {
    QString stem = QFileInfo(masterPath).completeBaseName();
    if (stem.isEmpty()) {
        const deliver::Output master = deliver::output(projectDirectory(), QStringLiteral("master"));
        stem = master.name;
    }
    // "atlas_master" and "atlas_master 2" give "atlas" and "atlas 2".
    if (const qsizetype at = stem.indexOf(QStringLiteral("_master")); at > 0) stem.remove(at, 7);
    if (stem.isEmpty()) stem = deliver::projectName(projectDirectory());
    return stem;
}

QString PaperworkView::fileFor(const QString &id, const QString &masterPath) const {
    const QString base = baseName(masterPath.isEmpty() ? m_lastMaster : masterPath);
    if (base.isEmpty()) return {};
    if (id == kCsv) return base + QStringLiteral("_notes.csv");
    if (id == kEdl) return base + QStringLiteral("_locators.edl");
    if (id == kQc) return base + QStringLiteral("_qc.pdf");
    if (id == kCaptions) return base + QStringLiteral("_captions.srt");
    return {};
}

int PaperworkView::noteCount() const {
    const bool resolved = isOn(kResolved);
    if (m_bus && m_bus->review()) {
        const ReviewModel::Counts counts = m_bus->review()->counts();
        return resolved ? counts.all : counts.open;
    }
    if (!m_bus) return 0;
    // Before a review model exists: the threads of the last comments event.
    int count = 0;
    for (const QJsonValue &value : m_bus->lastEvent(QStringLiteral("comments")).value(QStringLiteral("items")).toArray()) {
        const QJsonObject comment = value.toObject();
        if (!comment.value(QStringLiteral("parent")).toString().isEmpty()) continue;
        if (resolved || !comment.value(QStringLiteral("resolved")).toBool()) ++count;
    }
    return count;
}

QString PaperworkView::reviewLinkText() const {
    // The comments' Copy link's form: the folder's slug and the revision.
    return links::make(projectDirectory(), m_bus ? qint64(m_bus->revision()) : 0);
}

QString PaperworkView::reviewLink() const {
    QString link = reviewLinkText();
    if (link.isEmpty() || !m_bus) return link;
    // Where a reviewer finds that revision: the branch, the commit and the remote.
    const QJsonObject status = m_bus->lastEvent(QStringLiteral("collab")).value(QStringLiteral("status")).toObject();
    QStringList query;
    for (const char *key : {"branch", "head", "remote"}) {
        const QString value = status.value(QLatin1String(key)).toString();
        if (!value.isEmpty())
            query << QStringLiteral("%1=%2").arg(QLatin1String(key), QString::fromUtf8(QUrl::toPercentEncoding(value)));
    }
    if (!query.isEmpty()) link += QLatin1Char('?') + query.join(QLatin1Char('&'));
    return link;
}

bool PaperworkView::copyReviewLink() {
    const QString link = reviewLink();
    if (link.isEmpty()) return false;
    if (QClipboard *clipboard = QGuiApplication::clipboard()) clipboard->setText(link);
    setStatus(kLink, QStringLiteral("Copied"), false);
    return true;
}

void PaperworkView::setStatus(const QString &id, const QString &text, bool bad) {
    m_status.insert(id, text);
    m_bad.insert(id, bad);
    rebuild();
}

void PaperworkView::rebuild() {
    const bool captions = hasCaptions();
    for (OptionRow *row : std::as_const(m_rows)) {
        row->setVisible(row->id != kCaptions || captions);
        if (row->box) {
            const QSignalBlocker blocker(row->box);
            row->box->setChecked(isOn(row->id));
        }
        QString detail;
        if (row->id == kMarkers) {
            const int notes = noteCount();
            detail = notes == 1 ? QStringLiteral("1 note") : QStringLiteral("%1 notes").arg(notes);
        } else if (row->id == kLink) {
            detail = reviewLinkText();
            row->setEnabled(!detail.isEmpty());
            if (detail.isEmpty()) detail = QStringLiteral("No revision yet");
        } else {
            detail = fileFor(row->id);
        }
        const QString status = m_status.value(row->id);
        row->bad = m_bad.value(row->id);
        if (row->bad) detail = status;
        else if (!status.isEmpty()) detail = detail.isEmpty() ? status : QStringLiteral("%1 · %2").arg(detail, status.toLower());
        row->detail = detail;
        row->update();
    }
    layoutRows();
}

void PaperworkView::layoutRows() {
    int y = 0;
    for (OptionRow *row : std::as_const(m_rows)) {
        if (row->isHidden()) continue;
        row->setGeometry(0, y, width(), kRowHeight);
        y += kRowHeight;
    }
}

void PaperworkView::resizeEvent(QResizeEvent *) {
    layoutRows();
}

void PaperworkView::deliver(const QString &masterPath) {
    if (!m_bus || masterPath.isEmpty()) return;
    m_lastMaster = masterPath;
    m_pending.clear();
    m_status.clear();
    m_bad.clear();
    const QDir folder = QFileInfo(masterPath).absoluteDir();
    const auto send = [this](const QString &id, const QString &command, const QString &format, const QString &path) {
        QJsonObject request{{QStringLiteral("type"), command}, {QStringLiteral("format"), format}};
        if (command == QStringLiteral("export_comments")) request.insert(QStringLiteral("includeResolved"), isOn(kResolved));
        if (!path.isEmpty()) request.insert(QStringLiteral("path"), path);
        if (m_bus->send(request)) {
            m_pending.append({id, command, format, path});
            m_status.insert(id, QStringLiteral("Writing…"));
        } else {
            m_status.insert(id, QStringLiteral("Not sent: Kerf is offline"));
            m_bad.insert(id, true);
        }
    };
    if (isOn(kMarkers)) send(kMarkers, QStringLiteral("export_comments"), QStringLiteral("markers"), QString());
    if (isOn(kCsv)) send(kCsv, QStringLiteral("export_comments"), QStringLiteral("csv"), folder.filePath(fileFor(kCsv, masterPath)));
    if (isOn(kEdl)) send(kEdl, QStringLiteral("export_comments"), QStringLiteral("edl"), folder.filePath(fileFor(kEdl, masterPath)));
    if (isOn(kCaptions) && hasCaptions())
        send(kCaptions, QStringLiteral("export_captions"), QStringLiteral("srt"), folder.filePath(fileFor(kCaptions, masterPath)));
    if (isOn(kQc)) {
        if (m_check.isEmpty()) {
            m_status.insert(kQc, QStringLiteral("Run QC first: no check to report"));
            m_bad.insert(kQc, true);
        } else {
            QString error;
            const QString path = folder.filePath(fileFor(kQc, masterPath));
            if (writeQcReport(path, m_check, m_bus->lastEvent(QStringLiteral("loudness")), reportInfo(masterPath), &error)) {
                m_status.insert(kQc, QStringLiteral("Written"));
            } else {
                m_status.insert(kQc, error);
                m_bad.insert(kQc, true);
            }
        }
    }
    rebuild();
}

void PaperworkView::handleEvent(const QString &type, const QJsonObject &event) {
    if (type == QStringLiteral("check")) {
        if (event.value(QStringLiteral("status")).toString() == QStringLiteral("done")) m_check = event;
        return;
    }
    if (type == QStringLiteral("comments") || type == QStringLiteral("collab")) {
        rebuild();
        return;
    }
    if (type != QStringLiteral("export") && type != QStringLiteral("error")) return;
    QString command;
    if (type == QStringLiteral("export")) {
        const QString kind = event.value(QStringLiteral("kind")).toString();
        if (kind == QStringLiteral("paperwork")) command = QStringLiteral("export_comments");
        else if (kind == QStringLiteral("captions")) command = QStringLiteral("export_captions");
        else return;
    } else {
        command = event.value(QStringLiteral("command")).toString();
        if (command != QStringLiteral("export_comments") && command != QStringLiteral("export_captions")) return;
    }
    // The request this answers: the one of its path, else of its format.
    // An answer that names either and matches no request is not about one
    // still pending (the bridge follows a failed `export` with an `error`
    // of the same format), so it is left alone. Only an answer naming
    // neither goes to the oldest of its command (the bridge answers in
    // order).
    const QString path = event.value(QStringLiteral("path")).toString();
    const QString format = event.value(QStringLiteral("format")).toString();
    qsizetype found = -1;
    for (qsizetype i = 0; i < m_pending.size() && found < 0; ++i)
        if (!path.isEmpty() && QDir::cleanPath(m_pending[i].path) == QDir::cleanPath(path)) found = i;
    for (qsizetype i = 0; i < m_pending.size() && found < 0; ++i)
        if (!format.isEmpty() && m_pending[i].command == command && m_pending[i].format == format) found = i;
    for (qsizetype i = 0; i < m_pending.size() && found < 0 && path.isEmpty() && format.isEmpty(); ++i)
        if (m_pending[i].command == command) found = i;
    if (found < 0) return;
    const QString status = event.value(QStringLiteral("status")).toString();
    if (type == QStringLiteral("export") && status != QStringLiteral("completed") && status != QStringLiteral("failed")) return;
    const Pending pending = m_pending.takeAt(found);
    if (type == QStringLiteral("error") || status == QStringLiteral("failed")) {
        QString message = event.value(QStringLiteral("message")).toString();
        if (message.isEmpty()) message = event.value(QStringLiteral("error")).toString(QStringLiteral("Failed"));
        setStatus(pending.id, message, true);
    } else {
        setStatus(pending.id, pending.id == kMarkers ? QStringLiteral("Added") : QStringLiteral("Written"), false);
    }
}

QJsonObject PaperworkView::reportInfo(const QString &masterPath) const {
    QJsonObject info{{QStringLiteral("project"), deliver::projectTitle(projectDirectory(), m_bus ? m_bus->timeline() : QJsonObject())},
                     {QStringLiteral("master"), QFileInfo(masterPath).fileName()},
                     {QStringLiteral("created"), QDateTime::currentDateTimeUtc().toString(Qt::ISODate)}};
    if (m_bus) {
        const QJsonObject sequence = deliver::masterSequence(m_bus->timeline());
        info.insert(QStringLiteral("sequence"), sequence.value(QStringLiteral("name")).toString());
        info.insert(QStringLiteral("revision"), double(m_bus->revision()));
        const double fps = sequence.value(QStringLiteral("fps")).toDouble();
        info.insert(QStringLiteral("fps"), fps > 0 ? fps : m_bus->fps());
    }
    return info;
}

bool PaperworkView::writeQcReport(const QString &path, const QJsonObject &check, const QJsonObject &loudness,
                                  const QJsonObject &info, QString *error) {
    const auto fail = [error](const QString &message) {
        if (error) *error = message;
        return false;
    };
    if (path.isEmpty()) return fail(QStringLiteral("No file to write the QC report to"));
    if (!QDir().mkpath(QFileInfo(path).absolutePath())) return fail(QStringLiteral("Cannot make %1").arg(QFileInfo(path).absolutePath()));
    const double fps = info.value(QStringLiteral("fps")).toDouble(25);
    const QJsonObject summary = check.value(QStringLiteral("summary")).toObject();

    QString html;
    html += QStringLiteral("<h1>QC report</h1>");
    QStringList about;
    for (const char *key : {"project", "sequence"})
        if (const QString value = info.value(QLatin1String(key)).toString(); !value.isEmpty()) about << cell(value);
    if (const qint64 revision = qint64(info.value(QStringLiteral("revision")).toDouble()); revision > 0)
        about << QStringLiteral("rev %1").arg(revision);
    if (const QString master = info.value(QStringLiteral("master")).toString(); !master.isEmpty()) about << cell(master);
    html += QStringLiteral("<p>%1</p>").arg(about.join(QStringLiteral(" · ")));
    if (const QString created = info.value(QStringLiteral("created")).toString(); !created.isEmpty())
        html += QStringLiteral("<p>Written %1</p>").arg(cell(created));

    html += QStringLiteral("<h2>Checks</h2>");
    const int passed = summary.value(QStringLiteral("passed")).toInt();
    const int total = summary.value(QStringLiteral("total")).toInt();
    const int blocking = summary.value(QStringLiteral("blocking")).toInt();
    html += QStringLiteral("<p>%1 of %2 pass · %3</p>")
                .arg(passed)
                .arg(total)
                .arg(blocking == 0 ? QStringLiteral("nothing blocks the delivery")
                                   : blocking == 1 ? QStringLiteral("1 blocking finding")
                                                   : QStringLiteral("%1 blocking findings").arg(blocking));
    html += QStringLiteral("<table border='1' cellspacing='0' cellpadding='4' width='100%'>"
                           "<tr><th align='left'>Check</th><th align='left'>Measured</th><th align='left'>Limit</th>"
                           "<th align='left'>Result</th></tr>");
    for (const QJsonValue &value : summary.value(QStringLiteral("rows")).toArray()) {
        const QJsonObject row = value.toObject();
        QString at;
        if (row.value(QStringLiteral("at")).isDouble()) at = QStringLiteral(" at ") + timelineTimecode(row.value(QStringLiteral("at")).toDouble(), fps);
        html += QStringLiteral("<tr><td>%1</td><td>%2%3</td><td>%4</td><td>%5</td></tr>")
                    .arg(cell(row.value(QStringLiteral("name")).toString(row.value(QStringLiteral("code")).toString())),
                         cell(row.value(QStringLiteral("value")).toVariant().toString()), cell(at),
                         cell(row.value(QStringLiteral("limit")).toVariant().toString()),
                         row.value(QStringLiteral("passed")).toBool() ? QStringLiteral("Pass") : QStringLiteral("<b>Fail</b>"));
    }
    html += QStringLiteral("</table>");

    html += QStringLiteral("<h2>Programme loudness (EBU R128)</h2>");
    if (loudness.value(QStringLiteral("status")).toString() == QStringLiteral("ready")) {
        const QJsonArray peak = loudness.value(QStringLiteral("truePeak")).toArray();
        html += QStringLiteral("<table border='1' cellspacing='0' cellpadding='4'>"
                               "<tr><td>Integrated</td><td>%1 LUFS</td></tr>"
                               "<tr><td>Loudness range</td><td>%2 LU</td></tr>"
                               "<tr><td>True peak L</td><td>%3 dBTP</td></tr>"
                               "<tr><td>True peak R</td><td>%4 dBTP</td></tr>"
                               "<tr><td>Target</td><td>%5 LUFS · %6 dBTP</td></tr></table>")
                    .arg(number(loudness.value(QStringLiteral("integrated")), 1), number(loudness.value(QStringLiteral("lra")), 1),
                         number(peak.at(0), 1), number(peak.at(1), 1), number(loudness.value(QStringLiteral("target")), 0),
                         number(loudness.value(QStringLiteral("limit")), 0));
    } else {
        html += QStringLiteral("<p>The programme was not measured.</p>");
    }

    html += QStringLiteral("<h2>Findings</h2>");
    const QJsonArray findings = check.value(QStringLiteral("findings")).toArray();
    if (findings.isEmpty()) {
        html += QStringLiteral("<p>None.</p>");
    } else {
        html += QStringLiteral("<table border='1' cellspacing='0' cellpadding='4' width='100%'>"
                               "<tr><th align='left'>Severity</th><th align='left'>At</th><th align='left'>Check</th>"
                               "<th align='left'>Finding</th></tr>");
        for (const QJsonValue &value : findings) {
            const QJsonObject finding = value.toObject();
            const QString at = finding.value(QStringLiteral("at")).isDouble()
                                   ? timelineTimecode(finding.value(QStringLiteral("at")).toDouble(), fps)
                                   : QString();
            html += QStringLiteral("<tr><td>%1</td><td>%2</td><td>%3</td><td>%4</td></tr>")
                        .arg(cell(finding.value(QStringLiteral("severity")).toString()), cell(at),
                             cell(finding.value(QStringLiteral("code")).toString()), cell(finding.value(QStringLiteral("message")).toString()));
        }
        html += QStringLiteral("</table>");
    }

    QSaveFile file(path);
    if (!file.open(QIODevice::WriteOnly)) return fail(QStringLiteral("Cannot write %1: %2").arg(path, file.errorString()));
    {
        QPdfWriter writer(&file);
        writer.setPageSize(QPageSize(QPageSize::A4));
        writer.setResolution(300);
        writer.setTitle(QStringLiteral("QC report"));
        writer.setCreator(QStringLiteral("Kerf"));
        QTextDocument document;
        QFont base = tokens::font(control);
        base.setPointSizeF(10);
        document.setDefaultFont(base);
        document.setHtml(html);
        document.print(&writer);
    }
    if (!file.commit()) return fail(QStringLiteral("Cannot write %1: %2").arg(path, file.errorString()));
    return true;
}

} // namespace kerf
