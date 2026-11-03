#include "deliver_tray.h"
#include "components.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "loudness_view.h"
#include "paperwork_view.h"
#include "qc_view.h"
#include "render_queue_view.h"
#include "review_model.h"
#include "tray_shell.h"

#include <QAction>
#include <QDir>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QLabel>
#include <QPainter>
#include <QSet>
#include <QTimer>

#include <cmath>

namespace kerf {

using shell::TrayPane;
using shell::text;

namespace {

// Spec 13 at 1600: the pane widths, on 8 pt gaps; Loudness keeps 240 and
// the others share the rest 552 : 404 : 364.
constexpr int kMasters = 552;
constexpr int kQc = 404;
constexpr int kLoudness = 240;
constexpr int kPaperwork = 364;
constexpr int kGap = tokens::space::s8;
// Panes start 8 under the bar and end 8 above the band's foot.
constexpr int kPaneTop = tokens::layout::strip + kGap;

} // namespace

DeliverTray::DeliverTray(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("deliverTray"));
    m_bar = new QWidget(this);
    m_bar->setObjectName(QStringLiteral("deliverBar"));
    auto *barLayout = new QHBoxLayout(m_bar);
    barLayout->setContentsMargins(tokens::space::panelLeft, 0, tokens::space::panelHeaderRight, 0);
    barLayout->setSpacing(0);
    barLayout->addWidget(text(QStringLiteral("Delivery"), tokens::controlStrong, tokens::ink));
    barLayout->addSpacing(tokens::space::s4);
    m_project = text(QString(), tokens::meta, tokens::ink3);
    m_project->setObjectName(QStringLiteral("deliverProject"));
    barLayout->addWidget(m_project);
    barLayout->addStretch(1);
    m_blockingLabel = text(QString(), tokens::meta, tokens::ink3);
    m_blockingLabel->setObjectName(QStringLiteral("deliverBlocking"));
    barLayout->addWidget(m_blockingLabel);
    barLayout->addSpacing(tokens::space::s8);
    m_render = new ui::Button(QStringLiteral("Render master"), ui::Button::Kind::Secondary);
    m_render->setObjectName(QStringLiteral("deliverRender"));
    connect(m_render, &QAbstractButton::clicked, this, &DeliverTray::renderMaster);
    barLayout->addWidget(m_render);

    m_queueView = new RenderQueueView(nullptr, bus);
    m_qc = new QcView(nullptr, bus);
    m_loudness = new LoudnessView(nullptr, bus);
    m_paperwork = new PaperworkView(nullptr, bus);
    m_mastersMeta = text(QString(), tokens::meta, tokens::ink3);
    m_qcMeta = text(QString(), tokens::meta, tokens::ink3);
    const struct {
        const char *id;
        const char *name;
        QLabel *meta;
        QWidget *body;
    } specs[] = {{"masters", "Masters", m_mastersMeta, m_queueView},
                 {"qc", "Preflight QC", m_qcMeta, m_qc},
                 {"loudness", "Loudness", text(QStringLiteral("Programme"), tokens::meta, tokens::ink3), m_loudness},
                 {"paperwork", "Paperwork", text(QStringLiteral("travels with the master"), tokens::meta, tokens::ink3), m_paperwork}};
    for (const auto &spec : specs) {
        const QString id = QString::fromLatin1(spec.id);
        auto *pane = new TrayPane(QStringLiteral("deliverPane_%1").arg(id), QString::fromLatin1(spec.name), spec.meta, spec.body, this);
        m_panes.insert(id, pane);
    }
    connect(m_queueView, &RenderQueueView::rowsChanged, this, [this] { m_mastersMeta->setText(m_queueView->summaryText()); });
    connect(m_queueView, &RenderQueueView::jobCompleted, this, &DeliverTray::masterCompleted);
    connect(deliver::Settings::instance(), &deliver::Settings::changed, this, [this](const QString &directory) {
        if (deliver::projectName(directory) == deliver::projectName(projectDirectory())) updateBar();
    });
    if (m_bus) {
        connect(m_bus, &EditorBus::highlightPane, this, &DeliverTray::highlightPane);
        connect(m_bus, &EditorBus::snapshotChanged, this, &DeliverTray::updateBar);
        connect(m_bus, &EditorBus::reviewChanged, this, &DeliverTray::watchReview);
        connect(m_bus, &EditorBus::event, this, [this](const QString &type, const QJsonObject &event) {
            if (type == QStringLiteral("render_queue")) m_queue = event;
            else if (type == QStringLiteral("comments")) setComments(event);
            else if (type == QStringLiteral("check")) {
                if (event.value(QStringLiteral("status")).toString() == QStringLiteral("done")) m_check = event;
                updateBlocking();
            }
        });
        watchReview();
    }
    m_mastersMeta->setText(m_queueView->summaryText());
    updateBlocking();
}

void DeliverTray::setProjectDirectory(const QString &directory) {
    m_directory = directory;
    m_queueView->setProjectDirectory(directory);
    m_paperwork->setProjectDirectory(directory);
    updateBar();
}

QString DeliverTray::projectDirectory() const {
    return deliver::projectDirectory(this, m_directory);
}

void DeliverTray::setRenderAction(QAction *master) {
    if (m_renderAction) disconnect(m_renderAction, nullptr, this, nullptr);
    m_renderAction = master;
    if (master) connect(master, &QAction::changed, this, &DeliverTray::updateBar);
    m_queueView->setRenderAction(master);
    updateBar();
}

void DeliverTray::setQueue(const QJsonObject &queue) {
    m_queue = queue;
    m_queueView->setQueue(queue);
}

int DeliverTray::queueRowCount() const {
    return m_queueView->jobRowCount();
}

void DeliverTray::setLoudness(const QJsonObject &loudness) {
    m_loudness->setLoudness(loudness);
}

void DeliverTray::setCheck(const QJsonObject &check) {
    m_qc->setCheck(check);
    if (check.value(QStringLiteral("status")).toString() == QStringLiteral("done")) m_check = check;
    updateBlocking();
}

void DeliverTray::highlightPane(const QString &requested) {
    const QString id = requested == QStringLiteral("queue") ? QStringLiteral("masters") : requested;
    if (!m_panes.contains(id)) return;
    m_highlighted = id;
    for (auto it = m_panes.cbegin(); it != m_panes.cend(); ++it)
        static_cast<TrayPane *>(it.value())->setHighlighted(it.key() == id);
    QTimer::singleShot(1600, this, [this, id] {
        if (m_highlighted != id) return;
        m_highlighted.clear();
        static_cast<TrayPane *>(m_panes.value(id))->setHighlighted(false);
    });
}

QString DeliverTray::blockingText() const {
    return m_blockingLabel->text();
}

QString DeliverTray::projectText() const {
    return m_project->text();
}

void DeliverTray::watchReview() {
    if (m_review) disconnect(m_review, nullptr, this, nullptr);
    m_review = m_bus ? m_bus->review() : nullptr;
    if (auto *review = qobject_cast<ReviewModel *>(m_review.data()))
        connect(review, &ReviewModel::countsChanged, this, &DeliverTray::updateBlocking);
    updateBlocking();
}

void DeliverTray::setComments(const QJsonObject &comments) {
    m_comments = comments;
    m_commentsSeen = true;
    updateBlocking();
}

void DeliverTray::updateBlocking() {
    int blocking = -1;
    if (m_commentsSeen) {
        if (auto *review = qobject_cast<ReviewModel *>(m_review.data())) {
            blocking = review->counts().blocking;
        } else {
            // Threads with an open blocking note.
            QSet<QString> threads;
            for (const QJsonValue &value : m_comments.value(QStringLiteral("items")).toArray()) {
                const QJsonObject comment = value.toObject();
                if (!comment.value(QStringLiteral("blocking")).toBool() || comment.value(QStringLiteral("resolved")).toBool()) continue;
                const QString parent = comment.value(QStringLiteral("parent")).toString();
                threads.insert(parent.isEmpty() ? comment.value(QStringLiteral("id")).toString() : parent);
            }
            blocking = int(threads.size());
        }
        // Blocking findings no comment mirrors (past the bridge's cap of
        // new qc comments a check makes) block too.
        for (const QJsonValue &value : m_check.value(QStringLiteral("findings")).toArray()) {
            const QJsonObject finding = value.toObject();
            if (finding.value(QStringLiteral("severity")).toString() == QStringLiteral("error") &&
                finding.value(QStringLiteral("commentId")).toString().isEmpty())
                ++blocking;
        }
    }
    m_blocking = blocking;
    m_queueView->setBlocking(blocking);
    updateBar();
}

void DeliverTray::updateBar() {
    const QString directory = projectDirectory();
    QStringList parts;
    // The document's name for the project, else its folder's.
    if (const QString project = deliver::projectTitle(directory, m_bus ? m_bus->timeline() : QJsonObject()); !project.isEmpty())
        parts << project;
    if (m_bus) {
        if (const QString sequence = deliver::masterSequence(m_bus->timeline()).value(QStringLiteral("name")).toString(); !sequence.isEmpty())
            parts << sequence;
        if (m_bus->revision() > 0) parts << QStringLiteral("rev %1").arg(m_bus->revision());
    }
    if (const QString folder = deliver::output(directory, QStringLiteral("master")).folder; !folder.isEmpty()) parts << folder;
    m_project->setText(parts.join(QStringLiteral(" · ")));
    // Nothing is said of the notes before the first `comments` event.
    m_blockingLabel->setText(m_blocking < 0    ? QString()
                             : m_blocking == 0 ? QStringLiteral("No blocking notes")
                             : m_blocking == 1 ? QStringLiteral("1 blocking note open")
                                               : QStringLiteral("%1 blocking notes open").arg(m_blocking));
    // A local sheet, so the app's sheet (polished already) cannot keep the old ink.
    m_blockingLabel->setStyleSheet(
        QStringLiteral("QLabel { color: %1; }").arg(tokens::color(m_blocking > 0 ? tokens::neg : tokens::ink3).name()));
    // Render master is the registry's export.master (enabled with it, its
    // tip the registry's: the keymap's shortcut, why it is unavailable),
    // held back until the notes are known and while one blocks.
    shell::followAction(m_render, m_renderAction,
                        m_blocking < 0   ? QStringLiteral("Waiting for the review notes")
                        : m_blocking > 0 ? QStringLiteral("Resolve blocking notes first")
                                         : QString());
    // Primary once nothing blocks it.
    m_render->setKind(m_render->isEnabled() ? ui::Button::Kind::Primary : ui::Button::Kind::Secondary);
    m_render->setProperty("primary", m_render->isEnabled());
    m_qcMeta->setText(m_qc->summaryText());
}

bool DeliverTray::renderMaster() {
    if (!m_bus || m_blocking != 0 || !m_render->isEnabled()) return false;
    return m_queueView->start(QStringLiteral("master"));
}

void DeliverTray::masterCompleted(const QString &output, const QJsonObject &job) {
    if (output != QStringLiteral("master")) return;
    QString path = job.value(QStringLiteral("outputPath")).toString();
    if (path.isEmpty()) {
        // Where the master was asked to go.
        const QString directory = projectDirectory();
        const deliver::Output master = deliver::output(directory, QStringLiteral("master"));
        const QString file = deliver::plannedFile(master);
        if (directory.isEmpty() || file.isEmpty()) return;
        const QString folder = QDir::isAbsolutePath(master.folder) ? master.folder : QDir(directory).filePath(master.folder);
        path = QDir(folder).filePath(file);
    }
    m_paperwork->deliver(path);
}

void DeliverTray::layoutPanes() {
    const int rest = std::max(0, width() - 5 * kGap - kLoudness);
    const int shared = kMasters + kQc + kPaperwork;
    const int masters = int(std::lround(double(rest) * kMasters / shared));
    const int qc = int(std::lround(double(rest) * kQc / shared));
    const int paperwork = rest - masters - qc;
    const int h = std::max(0, height() - kPaneTop - kGap);
    int x = kGap;
    const struct {
        const char *id;
        int w;
    } order[] = {{"masters", masters}, {"qc", qc}, {"loudness", kLoudness}, {"paperwork", paperwork}};
    for (const auto &pane : order) {
        m_panes.value(QString::fromLatin1(pane.id))->setGeometry(x, kPaneTop, pane.w, h);
        x += pane.w + kGap;
    }
}

QSize DeliverTray::sizeHint() const {
    return {tokens::layout::canvasWidth, tokens::layout::band};
}

QSize DeliverTray::minimumSizeHint() const {
    // The fixed Loudness pane and the gaps, the bar and a pane header.
    return {kLoudness + 5 * kGap, kPaneTop + tokens::height::h28 + kGap};
}

void DeliverTray::resizeEvent(QResizeEvent *) {
    m_bar->setGeometry(0, 0, width(), tokens::layout::strip);
    layoutPanes();
}

void DeliverTray::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), tokens::color(tokens::chrome));
    // The bar's top and bottom lines.
    painter.fillRect(QRect(0, 0, width(), 1), tokens::color(tokens::line));
    painter.fillRect(QRect(0, tokens::layout::strip - 1, width(), 1), tokens::color(tokens::line));
}

} // namespace kerf
