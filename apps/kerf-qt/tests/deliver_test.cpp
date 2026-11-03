// Tests for the Deliver tray (mockup-spec 13) and the Inspector's Deliver
// pane (5.2): the masters queue from render_queue and export events, the
// Render master gate, the paperwork the finished master triggers, the QC
// report PDF, the review link, and the panes' geometry at 1600.
//
// Every fixture is shaped like the bridge's events (plan section 3); a bus
// with a recording sender stands in for the window.

#include "test_registry.h"

#include "deliver_pane.h"
#include "deliver_tray.h"
#include "components.h"
#include "editor_bus.h"
#include "paperwork_view.h"
#include "qc_view.h"
#include "render_queue_view.h"
#include "review_model.h"

#include <QAction>
#include <QClipboard>
#include <QDir>
#include <QFile>
#include <QGuiApplication>
#include <QJsonArray>
#include <QJsonObject>
#include <QList>
#include <QObject>
#include <QRegularExpression>
#include <QSettings>
#include <QTemporaryDir>
#include <QTest>

#include <memory>

namespace kerf {

namespace {

// The design's 30 s spot: one 1920x1080 25p sequence, five video tracks,
// an audio track and, when asked, a subtitle track and a portrait sequence.
QJsonObject timeline(bool portrait = false, bool subtitles = false) {
    QJsonArray tracks;
    for (int i = 0; i < 5; ++i) tracks.append(QJsonObject{{"id", QStringLiteral("V%1").arg(i + 1)}, {"type", "video"}});
    tracks.append(QJsonObject{{"id", "A1"}, {"type", "audio"}});
    if (subtitles) tracks.append(QJsonObject{{"id", "S1"}, {"type", "subtitle"}});
    QJsonArray sequences{QJsonObject{{"id", "s1"}, {"name", "30 s spot v7"}, {"width", 1920}, {"height", 1080}, {"fps", 25.0}, {"duration", 30.0}}};
    if (portrait)
        sequences.append(QJsonObject{{"id", "s2"}, {"name", "Vertical"}, {"width", 1080}, {"height", 1920}, {"fps", 25.0}, {"duration", 30.0}});
    return QJsonObject{{"sequence", "s1"}, {"width", 1920},        {"height", 1080},  {"fps", 25.0},     {"rate", "25"},
                       {"sampleRate", 48000}, {"duration", 30.0}, {"tracks", tracks}, {"sequences", sequences}};
}

// A bus as MainWindow sets it up, recording what the regions send and run.
struct Harness {
    EditorBus bus;
    QList<QJsonObject> sent;
    QList<QString> ran;
    bool online = true;

    Harness() {
        bus.setSender([this](const QJsonObject &command) {
            if (!online) return false;
            sent << command;
            return true;
        });
        bus.setRunner([this](const QString &id, const QJsonObject &) {
            ran << id;
            return online;
        });
        setTimeline(timeline());
    }
    void setTimeline(const QJsonObject &program, quint64 revision = 22) {
        bus.setSnapshot(QJsonObject{{"revision", double(revision)}}, program, 25.0, QSize(1920, 1080));
    }
    void event(const QJsonObject &event) { bus.dispatch(event); }
    QList<QJsonObject> sentOf(const QString &type) const {
        QList<QJsonObject> found;
        for (const QJsonObject &command : sent)
            if (command.value("type").toString() == type) found << command;
        return found;
    }
};

QJsonObject comments(bool resolved) {
    return QJsonObject{{"type", "comments"}, {"reviewRevision", 4}, {"items", QJsonArray{
        QJsonObject{{"id", "c-1708"}, {"parent", QJsonValue()}, {"body", "Caption overlaps the lower third"}, {"blocking", true}, {"resolved", resolved}},
        QJsonObject{{"id", "c-1708-r"}, {"parent", "c-1708"}, {"body", "Agreed"}, {"blocking", true}, {"resolved", resolved}},
        QJsonObject{{"id", "c-0903"}, {"parent", QJsonValue()}, {"body", "Warmer"}, {"blocking", false}, {"resolved", false}}}}};
}

QJsonObject doneCheck(const QJsonArray &findings = {}) {
    return QJsonObject{{"type", "check"}, {"status", "done"}, {"valid", findings.isEmpty()}, {"findings", findings},
                       {"summary", QJsonObject{{"passed", 6}, {"total", 8}, {"blocking", 1}, {"rows", QJsonArray{
                           QJsonObject{{"code", "loudness.integrated"}, {"name", "Integrated loudness"}, {"passed", true}, {"value", "−23.1 LUFS"}, {"limit", "±0.5"}},
                           QJsonObject{{"code", "loudness.true_peak"}, {"name", "True peak"}, {"passed", false}, {"value", "−0.4 dBTP"}, {"limit", "≤ −1.0"}, {"commentId", "c-2718"}},
                           QJsonObject{{"code", "text.overlap"}, {"name", "Caption overlap"}, {"passed", false}, {"value", "1"}, {"at", 17.32}, {"commentId", "c-1708"}}}}}}};
}

QJsonObject job(const QString &id, const QString &name, const QString &quality, const QString &status, double progress = 0,
                const QString &path = {}, const QString &sequence = {}) {
    QJsonObject job{{"jobId", id}, {"name", name}, {"quality", quality}, {"codec", quality == "master" ? "prores_hq" : "h264"},
                    {"status", status}, {"progress", progress}};
    if (!path.isEmpty()) job.insert("outputPath", path);
    if (!sequence.isEmpty()) job.insert("sequence", sequence);
    return job;
}

QJsonObject queue(const QJsonArray &jobs) {
    return QJsonObject{{"type", "render_queue"}, {"jobs", jobs}};
}

int pdfPages(const QByteArray &pdf) {
    return int(pdf.count("/Type /Page") - pdf.count("/Type /Pages"));
}

} // namespace

class DeliverTest final : public QObject {
    Q_OBJECT

private:
    std::unique_ptr<QTemporaryDir> m_root;
    QString m_project;

private slots:
    // Each test starts with no delivery settings, in a fresh project "atlas".
    void init() {
        QSettings().remove(QStringLiteral("deliver"));
        QSettings().remove(QStringLiteral("viewer"));
        m_root = std::make_unique<QTemporaryDir>();
        QVERIFY(m_root->isValid());
        m_project = QDir(m_root->path()).filePath(QStringLiteral("atlas"));
        QVERIFY(QDir().mkpath(m_project));
    }

    void cleanup() { QSettings().remove(QStringLiteral("deliver")); }

    void mastersRowsComeFromTheOutputsAndTheQueue() {
        Harness h;
        DeliverTray tray(nullptr, &h.bus);
        tray.setProjectDirectory(m_project);
        RenderQueueView *view = tray.renderQueueView();
        // The configured outputs, with the spec line their settings make.
        QCOMPARE(view->rowIds(), (QStringList{"master", "draft", "preview"}));
        QCOMPARE(view->summaryText(), QStringLiteral("3 outputs"));
        QCOMPARE(view->titleText("master"), QStringLiteral("Master"));
        QCOMPARE(view->specText("master"), QStringLiteral("ProRes 422 HQ · 1920×1080 · 25p · Rec.709 · PCM 48 kHz 24-bit stereo"));
        QCOMPARE(view->specText("draft"), QStringLiteral("H.264 · 1920×1080 · 25p · 12 Mb/s · review copy"));
        QCOMPARE(view->specText("preview"), QStringLiteral("V1–V5 · 00:00:00:00–00:00:30:00"));
        QCOMPARE(view->statusText("master"), QStringLiteral("Not started"));
        QCOMPARE(view->statusText("preview"), QStringLiteral("Not rendered"));
        QCOMPARE(view->pathText(), QStringLiteral("atlas_master.mov → deliveries/"));
        QCOMPARE(tray.queueRowCount(), 0);

        // The preview's own status.
        h.event(QJsonObject{{"type", "preview"}, {"status", "running"}, {"progress", 0.5}});
        QCOMPARE(view->progress("preview"), 50);
        h.event(QJsonObject{{"type", "preview"}, {"status", "ready"}});
        QCOMPARE(view->statusText("preview"), QStringLiteral("Ready"));
        QVERIFY(view->specText("preview").endsWith(QStringLiteral(" · cached")));

        // The queue: the Draft renders, an export from elsewhere has a row of its own.
        h.event(queue({job("j1", "atlas_draft", "review", "running", 0.38),
                       job("j9", "", "full", "queued", 0, m_project + "/render-j9.mp4")}));
        QCOMPARE(view->statusText("draft"), QStringLiteral("38%"));
        QCOMPARE(view->progress("draft"), 38);
        QCOMPARE(view->rowIds(), (QStringList{"master", "draft", "preview", "job:j9"}));
        QCOMPARE(view->titleText("job:j9"), QStringLiteral("render-j9"));
        QCOMPARE(view->statusText("job:j9"), QStringLiteral("Queued"));
        QCOMPARE(tray.queueRowCount(), 2);
        QCOMPARE(view->summaryText(), QStringLiteral("4 outputs"));
        // Rows are 52 tall.
        tray.resize(1600, 344);
        QCOMPARE(view->row("draft")->height(), 52);

        // export events move a job between queue events.
        h.event(QJsonObject{{"type", "export"}, {"jobId", "j1"}, {"status", "running"}, {"progress", 0.5}});
        QCOMPARE(view->statusText("draft"), QStringLiteral("50%"));
        // Cancel sends export_cancel for the row's job.
        QVERIFY(view->cancel("draft"));
        QCOMPARE(h.sent.last(), (QJsonObject{{"type", "export_cancel"}, {"jobId", "j1"}}));
        // A failed export says why in `message`, as the bridge's queue sends it.
        h.event(QJsonObject{{"type", "export"}, {"jobId", "j1"}, {"status", "failed"}, {"message", "The render was refused"}});
        QCOMPARE(view->statusText("draft"), QStringLiteral("The render was refused"));
        QVERIFY(view->statusIsBad("draft"));
        QVERIFY(!view->cancel("draft"));
        h.event(QJsonObject{{"type", "export"}, {"jobId", "j1"}, {"status", "completed"}, {"progress", 1.0}});
        QCOMPARE(view->statusText("draft"), QStringLiteral("Done"));

        // Draft renders with its own settings.
        h.sent.clear();
        QVERIFY(view->start("draft"));
        QCOMPARE(h.sent.last(), (QJsonObject{{"type", "export"}, {"quality", "review"}, {"output", "deliveries/"}, {"name", "atlas_draft"}}));
    }

    void verticalCutAppearsOnlyWithAPortraitSequence() {
        Harness h;
        DeliverTray tray(nullptr, &h.bus);
        tray.setProjectDirectory(m_project);
        RenderQueueView *view = tray.renderQueueView();
        QVERIFY(!view->rowIds().contains("vertical"));
        QVERIFY(!view->start("vertical"));
        h.setTimeline(timeline(true));
        QCOMPARE(view->rowIds(), (QStringList{"master", "draft", "preview", "vertical"}));
        QCOMPARE(view->specText("vertical"), QStringLiteral("H.264 · 1080×1920 · 25p · Vertical"));
        QCOMPARE(view->statusText("vertical"), QStringLiteral("Not started"));
        // It renders that sequence.
        QVERIFY(view->start("vertical"));
        QCOMPARE(h.sent.last(), (QJsonObject{{"type", "export"}, {"quality", "full"}, {"sequence", "s2"},
                                             {"output", "deliveries/"}, {"name", "atlas_vertical"}}));
        h.event(queue({job("j2", "atlas_vertical", "full", "running", 0.2, {}, "s2")}));
        QCOMPARE(view->statusText("vertical"), QStringLiteral("20%"));
        // A portrait first sequence is the master itself, not a second cut.
        QJsonObject portraitOnly = timeline();
        portraitOnly["sequences"] = QJsonArray{QJsonObject{{"id", "s1"}, {"name", "Vertical"}, {"width", 1080}, {"height", 1920}, {"fps", 25.0}}};
        h.setTimeline(portraitOnly);
        QVERIFY(!view->rowIds().contains("vertical"));
    }

    void renderMasterIsHeldBackWhileANoteBlocks() {
        Harness h;
        DeliverTray tray(nullptr, &h.bus);
        tray.setProjectDirectory(m_project);
        QAction master(QStringLiteral("Export Master"));
        master.setToolTip(QStringLiteral("Export Master · ⌘E"));
        tray.setRenderAction(&master);
        QCOMPARE(tray.projectText(), QStringLiteral("atlas · 30 s spot v7 · rev 22 · deliveries/"));
        // The document's own name for the project comes first when it has one.
        QJsonObject named = timeline();
        named["name"] = "Atlas";
        h.setTimeline(named);
        QCOMPARE(tray.projectText(), QStringLiteral("Atlas · 30 s spot v7 · rev 22 · deliveries/"));
        QCOMPARE(tray.paperworkView()->reviewLinkText(), QStringLiteral("kerf://atlas/rev22"));
        // Before the notes are known, nothing is said of them and the master waits.
        QCOMPARE(tray.blockingCount(), -1);
        QVERIFY(tray.blockingText().isEmpty());
        QVERIFY(!tray.renderButton()->isEnabled());
        QCOMPARE(tray.renderButton()->toolTip(), QStringLiteral("Waiting for the review notes"));
        QVERIFY(!tray.renderMaster());
        // A blocking thread (its reply too is blocking: one thread).
        h.event(comments(false));
        QCOMPARE(tray.blockingCount(), 1);
        QCOMPARE(tray.blockingText(), QStringLiteral("1 blocking note open"));
        QVERIFY(!tray.renderButton()->isEnabled());
        QCOMPARE(tray.renderButton()->toolTip(), QStringLiteral("Resolve blocking notes first"));
        QVERIFY(tray.renderButton()->kind() == ui::Button::Kind::Secondary);
        QCOMPARE(tray.renderQueueView()->statusText("master"), QStringLiteral("1 blocking note open"));
        QVERIFY(tray.renderQueueView()->statusIsBad("master"));
        // Resolved, the master is the primary action.
        h.event(comments(true));
        QCOMPARE(tray.blockingText(), QStringLiteral("No blocking notes"));
        QVERIFY(tray.renderButton()->isEnabled());
        QVERIFY(tray.renderButton()->kind() == ui::Button::Kind::Primary);
        QCOMPARE(tray.renderButton()->toolTip(), master.toolTip());
        QCOMPARE(tray.renderQueueView()->statusText("master"), QStringLiteral("Not started"));
        // A blocking finding no comment mirrors blocks too; one with a comment is that comment's.
        h.event(doneCheck({QJsonObject{{"severity", "error"}, {"code", "loudness.true_peak"}, {"message", "True peak −0.4 dBTP"}, {"commentId", "c-2718"}},
                           QJsonObject{{"severity", "error"}, {"code", "picture.flashing"}, {"message", "Flashing at 00:00:21:04"}},
                           QJsonObject{{"severity", "warning"}, {"code", "caption.speed"}, {"message", "Fast caption"}}}));
        QCOMPARE(tray.blockingCount(), 1);
        QVERIFY(!tray.renderButton()->isEnabled());
        QCOMPARE(tray.qcView()->summaryText(), QStringLiteral("6 of 8 pass"));
        h.event(doneCheck());
        QCOMPARE(tray.blockingCount(), 0);
        // Render master renders the Master output with its settings.
        tray.renderButton()->click();
        QCOMPARE(h.sent.last(), (QJsonObject{{"type", "export"}, {"quality", "master"}, {"output", "deliveries/"}, {"name", "atlas_master"}}));
        // Offline, the registry says so.
        master.setEnabled(false);
        QVERIFY(!tray.renderButton()->isEnabled());
        QVERIFY(!tray.renderMaster());
    }

    void reviewModelGivesTheBlockingCount() {
        Harness h;
        ReviewModel review;
        h.bus.setReview(&review);
        DeliverTray tray(nullptr, &h.bus);
        QAction master(QStringLiteral("Export Master"));
        tray.setRenderAction(&master);
        // The model is fed after the tray hears the event: its counts follow.
        QObject::connect(&h.bus, &EditorBus::event, &review, [&review](const QString &type, const QJsonObject &event) { review.applyEvent(type, event); });
        h.event(comments(false));
        QCOMPARE(review.counts().blocking, 1);
        QCOMPARE(tray.blockingCount(), 1);
        h.event(comments(true));
        QCOMPARE(tray.blockingCount(), 0);
        QVERIFY(tray.renderButton()->isEnabled());
        // Without a project, Render master is the registry's own export.master.
        QVERIFY(tray.renderMaster());
        QCOMPARE(h.ran, (QList<QString>{"export.master"}));
        QVERIFY(h.sentOf("export").isEmpty());
    }

    void finishedMasterTakesThePaperworkBesideIt() {
        Harness h;
        h.setTimeline(timeline(false, true));
        DeliverTray tray(nullptr, &h.bus);
        tray.setProjectDirectory(m_project);
        PaperworkView *paper = tray.paperworkView();
        h.event(comments(true));
        h.event(doneCheck());
        h.event(QJsonObject{{"type", "loudness"}, {"status", "ready"}, {"integrated", -23.1}, {"lra", 6.2},
                            {"truePeak", QJsonArray{-0.4, -1.3}}, {"target", -23}, {"limit", -1}});
        // The rows, their files and counts.
        QCOMPARE(paper->rowIds(), (QStringList{"markers", "csv", "edlLocators", "includeResolved", "qcReport", "captions", "link"}));
        QCOMPARE(paper->detailText("markers"), QStringLiteral("1 note"));
        QCOMPARE(paper->detailText("csv"), QStringLiteral("atlas_notes.csv"));
        QCOMPARE(paper->detailText("edlLocators"), QStringLiteral("atlas_locators.edl"));
        QCOMPARE(paper->detailText("qcReport"), QStringLiteral("atlas_qc.pdf"));
        QVERIFY(paper->isOn("markers") && paper->isOn("csv") && paper->isOn("edlLocators") && paper->isOn("qcReport"));
        QVERIFY(!paper->isOn("includeResolved"));
        // A choice is kept for the project, where the toolbar reads it too.
        paper->setOn("includeResolved", true);
        QCOMPARE(QSettings().value("deliver/atlas/includeResolved").toBool(), true);
        QCOMPARE(paper->detailText("markers"), QStringLiteral("2 notes"));
        paper->setOn("edlLocators", false);
        QCOMPARE(QSettings().value("deliver/atlas/edlLocators").toBool(), false);
        paper->setOn("edlLocators", true);

        // A master that had finished before is not delivered again.
        h.event(queue({job("old", "atlas_master", "master", "completed", 1, m_project + "/deliveries/atlas_master.mov")}));
        QVERIFY(h.sentOf("export_comments").isEmpty());

        // The master renders, then finishes.
        const QString folder = QDir(m_project).filePath("deliveries");
        const QString path = QDir(folder).filePath("atlas_master 2.mov");
        h.event(queue({job("m1", "atlas_master", "master", "queued")}));
        h.event(queue({job("m1", "atlas_master", "master", "running", 0.6)}));
        QCOMPARE(tray.renderQueueView()->statusText("master"), QStringLiteral("60%"));
        QVERIFY(h.sentOf("export_comments").isEmpty());
        h.event(queue({job("m1", "atlas_master", "master", "completed", 1, path)}));
        QCOMPARE(tray.renderQueueView()->pathText(), QStringLiteral("atlas_master 2.mov → deliveries/"));
        const QList<QJsonObject> notes = h.sentOf("export_comments");
        QCOMPARE(notes.size(), 3);
        QCOMPARE(notes[0], (QJsonObject{{"type", "export_comments"}, {"format", "markers"}, {"includeResolved", true}}));
        QCOMPARE(notes[1], (QJsonObject{{"type", "export_comments"}, {"format", "csv"}, {"includeResolved", true},
                                        {"path", QDir(folder).filePath("atlas 2_notes.csv")}}));
        QCOMPARE(notes[2], (QJsonObject{{"type", "export_comments"}, {"format", "edl"}, {"includeResolved", true},
                                        {"path", QDir(folder).filePath("atlas 2_locators.edl")}}));
        const QList<QJsonObject> captions = h.sentOf("export_captions");
        QCOMPARE(captions.size(), 1);
        QCOMPARE(captions[0], (QJsonObject{{"type", "export_captions"}, {"format", "srt"}, {"path", QDir(folder).filePath("atlas 2_captions.srt")}}));
        // The QC report is written here, beside the master.
        const QString report = QDir(folder).filePath("atlas 2_qc.pdf");
        QVERIFY(QFile::exists(report));
        QCOMPARE(paper->statusText("qcReport"), QStringLiteral("Written"));
        QCOMPARE(paper->statusText("csv"), QStringLiteral("Writing…"));
        // Each answer says how its item went.
        h.event(QJsonObject{{"type", "export"}, {"status", "completed"}, {"kind", "paperwork"}, {"path", QDir(folder).filePath("atlas 2_notes.csv")}});
        QCOMPARE(paper->statusText("csv"), QStringLiteral("Written"));
        QCOMPARE(paper->detailText("csv"), QStringLiteral("atlas 2_notes.csv · written"));
        // A failure comes as the bridge sends it: a failed export of its
        // format, then an error of the same format. Only that row turns bad;
        // the EDL still pending keeps waiting for its own answer.
        h.event(QJsonObject{{"type", "export"}, {"status", "failed"}, {"kind", "paperwork"}, {"format", "markers"},
                            {"message", "The review log is busy"}});
        h.event(QJsonObject{{"type", "error"}, {"message", "The review log is busy"}, {"command", "export_comments"}, {"format", "markers"}});
        QCOMPARE(paper->statusText("markers"), QStringLiteral("The review log is busy"));
        QVERIFY(paper->statusIsBad("markers"));
        QCOMPARE(paper->detailText("markers"), QStringLiteral("The review log is busy"));
        QCOMPARE(paper->statusText("edlLocators"), QStringLiteral("Writing…"));
        QVERIFY(!paper->statusIsBad("edlLocators"));
        QVERIFY(!paper->statusIsBad("csv"));
        h.event(QJsonObject{{"type", "export"}, {"status", "completed"}, {"kind", "captions"}, {"format", "srt"},
                            {"path", QDir(folder).filePath("atlas 2_captions.srt")}});
        QCOMPARE(paper->statusText("captions"), QStringLiteral("Written"));
        QCOMPARE(paper->statusText("edlLocators"), QStringLiteral("Writing…"));
        // An answer of a format nothing waits for is left alone.
        h.event(QJsonObject{{"type", "export"}, {"status", "completed"}, {"kind", "paperwork"}, {"format", "csv"}});
        QCOMPARE(paper->statusText("edlLocators"), QStringLiteral("Writing…"));
        h.event(QJsonObject{{"type", "export"}, {"status", "completed"}, {"kind", "paperwork"}, {"format", "edl"},
                            {"path", QDir(folder).filePath("atlas 2_locators.edl")}});
        QCOMPARE(paper->statusText("edlLocators"), QStringLiteral("Written"));
        QCOMPARE(paper->statusText("markers"), QStringLiteral("The review log is busy"));
        // The Deliver pane names the paperwork chosen.
        DeliverPane pane(nullptr, &h.bus);
        pane.setProjectDirectory(m_project);
        QCOMPARE(pane.value("Destination", "Paperwork"), QStringLiteral("CSV · EDL · SRT · QC report"));
    }

    void paperworkWithoutACheckSaysSo() {
        Harness h;
        PaperworkView paper(nullptr, &h.bus);
        paper.setProjectDirectory(m_project);
        QVERIFY(!paper.rowIds().contains("captions"));
        paper.deliver(QDir(m_project).filePath("deliveries/atlas_master.mov"));
        QVERIFY(paper.statusIsBad("qcReport"));
        QCOMPARE(paper.statusText("qcReport"), QStringLiteral("Run QC first: no check to report"));
        QVERIFY(h.sentOf("export_captions").isEmpty());
        // Offline nothing is sent, and the rows say so.
        h.online = false;
        paper.deliver(QDir(m_project).filePath("deliveries/atlas_master.mov"));
        QVERIFY(paper.statusIsBad("csv"));
    }

    void qcReportIsAPdf() {
        const QString path = QDir(m_project).filePath("deliveries/atlas_qc.pdf");
        QString error;
        const QJsonObject loudness{{"status", "ready"}, {"integrated", -23.1}, {"lra", 6.2}, {"truePeak", QJsonArray{-0.4, -1.3}},
                                   {"target", -23}, {"limit", -1}};
        const QJsonArray findings{QJsonObject{{"severity", "error"}, {"code", "loudness.true_peak"}, {"message", "True peak −0.4 dBTP"}, {"at", 27.18}}};
        QVERIFY2(PaperworkView::writeQcReport(path, doneCheck(findings), loudness,
                                              QJsonObject{{"project", "atlas"}, {"sequence", "30 s spot v7"}, {"revision", 22},
                                                          {"master", "atlas_master.mov"}, {"fps", 25}},
                                              &error),
                 qPrintable(error));
        QFile file(path);
        QVERIFY(file.open(QIODevice::ReadOnly));
        const QByteArray pdf = file.readAll();
        QVERIFY(pdf.startsWith("%PDF"));
        QVERIFY2(pdfPages(pdf) >= 1, qPrintable(QString::number(pdfPages(pdf))));
        // Nothing to write to is refused, with a reason.
        QVERIFY(!PaperworkView::writeQcReport(QString(), doneCheck(), {}, {}, &error));
        QVERIFY(!error.isEmpty());
    }

    void reviewLinkNamesTheProjectAndRevision() {
        Harness h;
        PaperworkView paper(nullptr, &h.bus);
        paper.setProjectDirectory(m_project);
        QCOMPARE(paper.reviewLinkText(), QStringLiteral("kerf://atlas/rev22"));
        QCOMPARE(paper.detailText("link"), QStringLiteral("kerf://atlas/rev22"));
        QCOMPARE(paper.reviewLink(), QStringLiteral("kerf://atlas/rev22"));
        h.event(QJsonObject{{"type", "collab"}, {"status", QJsonObject{{"branch", "main"}, {"head", "4f2a9c1"},
                                                                       {"remote", "git@github.com:kerf/atlas.git"}}}});
        QCOMPARE(paper.reviewLink(),
                 QStringLiteral("kerf://atlas/rev22?branch=main&head=4f2a9c1&remote=git%40github.com%3Akerf%2Fatlas.git"));
        QVERIFY(paper.reviewLink().contains(QRegularExpression(QStringLiteral("^kerf://[^/?]+/rev\\d+(\\?|$)"))));
        QVERIFY(paper.copyReviewLink());
        QCOMPARE(QGuiApplication::clipboard()->text(), paper.reviewLink());
        h.setTimeline(timeline(), 23);
        QCOMPARE(paper.reviewLinkText(), QStringLiteral("kerf://atlas/rev23"));
        // No revision yet: nothing to link to.
        h.setTimeline(timeline(), 0);
        QVERIFY(paper.reviewLink().isEmpty());
        QVERIFY(!paper.copyReviewLink());
        QVERIFY(!paper.row("link")->isEnabled());
    }

    void deliverPaneShowsTheMastersSettings() {
        Harness h;
        DeliverPane pane(nullptr, &h.bus);
        // No project: the master is unnamed and its folder cannot be chosen.
        QCOMPARE(pane.fileText(), QStringLiteral("No master output yet"));
        QVERIFY(!pane.folderButton()->isEnabled());
        pane.setProjectDirectory(m_project);
        QCOMPARE(pane.titleText(), QStringLiteral("Master"));
        QCOMPARE(pane.fileText(), QStringLiteral("atlas_master.mov"));
        QCOMPARE(pane.value("Video", "Format"), QStringLiteral("QuickTime"));
        QCOMPARE(pane.value("Codec"), QStringLiteral("ProRes 422 HQ"));
        QCOMPARE(pane.value("Resolution"), QStringLiteral("1920×1080"));
        QCOMPARE(pane.value("Frame rate"), QStringLiteral("25p"));
        QCOMPARE(pane.value("Colour space"), QStringLiteral("Rec.709"));
        QCOMPARE(pane.value("Audio", "Format"), QStringLiteral("PCM 48 kHz 24-bit"));
        QCOMPARE(pane.value("Channels"), QStringLiteral("Stereo · R128 −23 LUFS"));
        QCOMPARE(pane.value("Folder"), QStringLiteral("deliveries/"));
        QCOMPARE(pane.value("Paperwork"), QStringLiteral("CSV · EDL · QC report"));
        QVERIFY(pane.folderButton()->isEnabled());
        // A folder chosen in the project is kept relative, and renders go there.
        DeliverTray tray(nullptr, &h.bus);
        tray.setProjectDirectory(m_project);
        QAction master(QStringLiteral("Export Master"));
        tray.setRenderAction(&master);
        h.event(comments(true));
        const QString finals = QDir(m_project).filePath("out/finals");
        QVERIFY(QDir().mkpath(finals));
        pane.setFolder(finals);
        QCOMPARE(pane.value("Folder"), QStringLiteral("out/finals/"));
        QCOMPARE(tray.projectText(), QStringLiteral("atlas · 30 s spot v7 · rev 22 · out/finals/"));
        QVERIFY(tray.renderMaster());
        QCOMPARE(h.sent.last().value("output").toString(), QStringLiteral("out/finals/"));
        // Draft follows the master's folder.
        QVERIFY(tray.renderQueueView()->start("draft"));
        QCOMPARE(h.sent.last().value("output").toString(), QStringLiteral("out/finals/"));
        // The master rendered there names the file; the folder is the one set.
        h.event(queue({job("m0", "atlas_master", "master", "completed", 1, QDir(finals).filePath("atlas_master 3.mov"))}));
        QCOMPARE(pane.fileText(), QStringLiteral("atlas_master 3.mov"));
        QCOMPARE(pane.value("Folder"), QStringLiteral("out/finals/"));
        QCOMPARE(tray.renderQueueView()->pathText(), QStringLiteral("atlas_master 3.mov → out/finals/"));
        // Another folder chosen since: the pane, the path row and the bar all
        // name it, with the file the next render plans.
        QTemporaryDir outside;
        const QString elsewhere = QDir::cleanPath(QFileInfo(outside.path()).absoluteFilePath());
        pane.setFolder(outside.path());
        QCOMPARE(pane.value("Folder"), elsewhere);
        QCOMPARE(pane.fileText(), QStringLiteral("atlas_master.mov"));
        QCOMPARE(tray.renderQueueView()->pathText(), QStringLiteral("atlas_master.mov → %1").arg(elsewhere));
        QCOMPARE(tray.projectText(), QStringLiteral("atlas · 30 s spot v7 · rev 22 · %1").arg(elsewhere));
        // The master rendered there names the file.
        h.event(queue({job("m0", "atlas_master", "master", "completed", 1, QDir(finals).filePath("atlas_master 3.mov")),
                       job("m1", "atlas_master", "master", "completed", 1, QDir(outside.path()).filePath("atlas_master 2.mov"))}));
        QCOMPARE(pane.fileText(), QStringLiteral("atlas_master 2.mov"));
        QCOMPARE(tray.renderQueueView()->pathText(), QStringLiteral("atlas_master 2.mov → %1").arg(elsewhere));
        // A master that failed or was cancelled made no file: the last
        // completed one still names it.
        h.event(queue({job("m1", "atlas_master", "master", "completed", 1, QDir(outside.path()).filePath("atlas_master 2.mov")),
                       job("m2", "atlas_master", "master", "failed", 0.4, QDir(outside.path()).filePath("atlas_master 4.mov")),
                       job("m3", "atlas_master", "master", "cancelled", 0.1, QDir(m_project).filePath("elsewhere/atlas_master.mov"))}));
        QCOMPARE(pane.fileText(), QStringLiteral("atlas_master 2.mov"));
        QCOMPARE(pane.value("Folder"), elsewhere);
        QCOMPARE(tray.renderQueueView()->pathText(), QStringLiteral("atlas_master 2.mov → %1").arg(elsewhere));
        // The measured target, when there is one.
        h.event(QJsonObject{{"type", "loudness"}, {"status", "ready"}, {"target", -24}});
        QCOMPARE(pane.value("Channels"), QStringLiteral("Stereo · R128 −24 LUFS"));
        // Highlighted over the bus.
        emit h.bus.highlightPane(QStringLiteral("deliver"));
        QVERIFY(pane.highlighted());
    }

    // kerfd keeps the queue's history across sessions: old previews and
    // renders from MCP or the agent that no output names do not fill the
    // pane, and the path row stays in view under the rows.
    void mastersPaneKeepsThePathRowInView() {
        Harness h;
        h.setTimeline(timeline(true));
        DeliverTray tray(nullptr, &h.bus);
        tray.setProjectDirectory(m_project);
        tray.resize(1600, 344);
        tray.show();
        QVERIFY(QTest::qWaitForWindowExposed(&tray));
        RenderQueueView *view = tray.renderQueueView();
        QWidget *pane = tray.pane("masters");
        const auto pathInPane = [&] {
            const QRect path = view->pathRowRect();
            if (path.isEmpty() || !view->rect().contains(path)) return false;
            const QRect inPane(view->mapTo(pane, path.topLeft()), path.size());
            return pane->rect().contains(inPane);
        };
        QJsonArray history;
        for (int i = 0; i < 6; ++i) {
            history.append(job(QStringLiteral("old%1").arg(i), "", "preview", "completed", 1, m_project + QStringLiteral("/.kerf/preview-%1.mp4").arg(i)));
            history.append(job(QStringLiteral("mcp%1").arg(i), QStringLiteral("agent cut %1").arg(i), "full", "completed", 1));
        }
        h.event(queue(history));
        QCOMPARE(view->rowIds(), (QStringList{"master", "draft", "preview", "vertical"}));
        QCOMPARE(view->summaryText(), QStringLiteral("4 outputs"));
        QVERIFY(pathInPane());
        // A render from elsewhere shows while it runs, and once it ends here
        // only the latest stays.
        history.append(job("agent1", "agent take", "full", "running", 0.3));
        h.event(queue(history));
        QCOMPARE(view->rowIds().last(), QStringLiteral("job:agent1"));
        QCOMPARE(view->statusText("job:agent1"), QStringLiteral("30%"));
        QVERIFY(pathInPane());
        history.append(job("agent2", "agent take 2", "full", "queued"));
        history[history.size() - 2] = job("agent1", "agent take", "full", "completed", 1);
        h.event(queue(history));
        QCOMPARE(view->rowIds().mid(4), (QStringList{"job:agent1", "job:agent2"}));
        history[history.size() - 1] = job("agent2", "agent take 2", "full", "completed", 1);
        h.event(queue(history));
        QCOMPARE(view->rowIds().mid(4), (QStringList{"job:agent2"}));
        // More active ones than fit scroll, and the path row stays under them.
        for (int i = 0; i < 4; ++i) history.append(job(QStringLiteral("busy%1").arg(i), QStringLiteral("busy %1").arg(i), "full", "queued"));
        h.event(queue(history));
        QCOMPARE(view->rowCount(), 9);
        QVERIFY(pathInPane());
        QCOMPARE(view->pathRowRect().bottom(), view->height() - 1);
        QCOMPARE(view->pathText(), QStringLiteral("atlas_master.mov → deliveries/"));
    }

    void panesSitAtTheSpecPlacesAt1600() {
        Harness h;
        DeliverTray tray(nullptr, &h.bus);
        tray.setProjectDirectory(m_project);
        tray.resize(1600, 344);
        tray.show();
        QVERIFY(QTest::qWaitForWindowExposed(&tray));
        const struct {
            const char *id;
            int x, w;
        } spec[] = {{"masters", 8, 552}, {"qc", 568, 404}, {"loudness", 980, 240}, {"paperwork", 1228, 364}};
        for (const auto &pane : spec) {
            const QRect frame = tray.pane(QString::fromLatin1(pane.id))->geometry();
            QCOMPARE(frame.x(), pane.x);
            QCOMPARE(frame.width(), pane.w);
            QCOMPARE(frame.y(), 36);
            QCOMPARE(frame.height(), 300);
        }
        // Paperwork rows are 32 tall, and the queue's body starts under the 28 header.
        QCOMPARE(tray.paperworkView()->row("csv")->height(), 32);
        QCOMPARE(tray.renderQueueView()->mapTo(&tray, QPoint(0, 0)).y(), 36 + 28);
        // Narrower, Loudness keeps 240 and the gaps stay 8.
        tray.resize(1366, 344);
        QCoreApplication::processEvents();
        QCOMPARE(tray.pane("loudness")->width(), 240);
        QCOMPARE(tray.pane("qc")->x() - tray.pane("masters")->geometry().right() - 1, 8);
        QCOMPARE(tray.pane("paperwork")->geometry().right(), 1366 - 8 - 1);
        // A pane asked for over the bus comes forward; the queue is the Masters pane.
        emit h.bus.highlightPane(QStringLiteral("queue"));
        QCOMPARE(tray.highlightedPane(), QStringLiteral("masters"));
    }

    // The tray and the pane with the design's fixture, drawn to
    // $KERF_DELIVER_GALLERY/deliver-tray.png and deliver-pane.png.
    void deliverGallery() {
        const QString folder = qEnvironmentVariable("KERF_DELIVER_GALLERY");
        if (folder.isEmpty()) QSKIP("Set KERF_DELIVER_GALLERY to draw the Deliver tray.");
        Harness h;
        h.setTimeline(timeline(true, true));
        DeliverTray tray(nullptr, &h.bus);
        tray.setProjectDirectory(m_project);
        QAction master(QStringLiteral("Export Master"));
        tray.setRenderAction(&master);
        h.event(comments(false));
        h.event(doneCheck());
        h.event(QJsonObject{{"type", "loudness"}, {"status", "ready"}, {"integrated", -23.1}, {"lra", 6.2},
                            {"truePeak", QJsonArray{-0.4, -1.3}}, {"target", -23}, {"limit", -1}});
        h.event(QJsonObject{{"type", "preview"}, {"status", "ready"}});
        h.event(queue({job("j1", "atlas_draft", "review", "running", 0.38)}));
        tray.resize(1600, 344);
        tray.show();
        QVERIFY(QTest::qWaitForWindowExposed(&tray));
        QVERIFY(tray.grab().save(QDir(folder).filePath("deliver-tray.png")));
        DeliverPane pane(nullptr, &h.bus);
        pane.setProjectDirectory(m_project);
        pane.resize(303, 480);
        pane.show();
        QVERIFY(QTest::qWaitForWindowExposed(&pane));
        QVERIFY(pane.grab().save(QDir(folder).filePath("deliver-pane.png")));
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::DeliverTest)

#include "deliver_test.moc"
