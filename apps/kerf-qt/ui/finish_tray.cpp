#include "finish_tray.h"
#include "components.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "editor_common.h"
#include "loudness_view.h"
#include "qc_view.h"
#include "tray_shell.h"

#include <QAction>
#include <QBoxLayout>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QLabel>
#include <QMenu>
#include <QPainter>
#include <QSettings>
#include <QTimer>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace {

// A line of a pane's header that gives way to what comes before it, with an ellipsis rather than
// cut off at the card's edge.
class ClippedMeta final : public QLabel {
public:
    QSize minimumSizeHint() const override { return QSize(0, QLabel::minimumSizeHint().height()); }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setFont(font());
        p.setPen(palette().color(QPalette::WindowText));
        p.drawText(rect(), Qt::AlignLeft | Qt::AlignVCenter, QFontMetrics(font()).elidedText(text(), Qt::ElideRight, width()));
    }
};

using shell::TrayPane;
using shell::text;

constexpr int kGap = tokens::space::s8;
constexpr int kFixedWidth = 240; // the vectorscope and loudness panes
// The waveform, parade and QC panes share the rest in this ratio.
constexpr int kShare[3] = {352, 352, 368};
constexpr int kShareTotal = kShare[0] + kShare[1] + kShare[2];
// Below the design's width QC's rows read before a scope needs its last pixels: QC has this much for
// as long as the scopes keep their least.
constexpr int kRoomyQc = 261;
constexpr int kLeastScope = 180;
const QString kModeKey = QStringLiteral("finish/waveformMode");

// "−23", "−1": a target as the design writes it.
QString signedNumber(double value) {
    const QString number = QString::number(std::abs(value), 'g', 3);
    return value < 0 ? QStringLiteral("−") + number : number;
}

// The colour a file is tagged with, by its primaries (else its matrix):
// "Rec.709", "Rec.2020". Untagged is empty.
QString colourName(const QJsonObject &info) {
    static const QHash<QString, QString> names{
        {QStringLiteral("bt709"), QStringLiteral("Rec.709")},       {QStringLiteral("bt2020"), QStringLiteral("Rec.2020")},
        {QStringLiteral("bt2020nc"), QStringLiteral("Rec.2020")},   {QStringLiteral("bt2020c"), QStringLiteral("Rec.2020")},
        {QStringLiteral("smpte432"), QStringLiteral("P3-D65")},     {QStringLiteral("smpte431"), QStringLiteral("DCI-P3")},
        {QStringLiteral("bt470bg"), QStringLiteral("Rec.601")},     {QStringLiteral("smpte170m"), QStringLiteral("Rec.601")},
    };
    const QString primaries = info.value(QStringLiteral("primaries")).toString();
    const QString space = info.value(QStringLiteral("colorSpace")).toString();
    const QString tag = primaries.isEmpty() ? space : primaries;
    return names.value(tag, tag);
}

// The clip the program shows at `seconds`: on the uppermost video track
// (the last in the timeline's list) that is on, its media or nested-sequence
// clip there that is on.
QJsonObject programClipAt(const QJsonObject &timeline, double seconds) {
    const QJsonArray tracks = timeline.value(QStringLiteral("tracks")).toArray();
    for (qsizetype i = tracks.size() - 1; i >= 0; --i) {
        const QJsonObject track = tracks.at(i).toObject();
        if (track.value(QStringLiteral("type")).toString() != QStringLiteral("video")) continue;
        if (!track.value(QStringLiteral("enabled")).toBool(true)) continue;
        for (const QJsonValue &value : track.value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = value.toObject();
            const QString kind = clip.value(QStringLiteral("kind")).toString(QStringLiteral("media"));
            if (kind != QStringLiteral("media") && kind != QStringLiteral("sequence")) continue;
            if (!clip.value(QStringLiteral("enabled")).toBool(true)) continue;
            const double start = clip.value(QStringLiteral("timelineStart")).toDouble();
            const double duration = clip.value(QStringLiteral("duration")).toDouble();
            if (seconds + 1e-9 >= start && seconds < start + duration) return clip;
        }
    }
    return {};
}

// What the bar calls `clip`: its own name, else its media's name as the bin
// gives it (clip_json sends a name only when one is set).
QString programClipName(const QJsonObject &timeline, const QJsonObject &clip) {
    const QString name = clip.value(QStringLiteral("name")).toString().trimmed();
    if (!name.isEmpty()) return name;
    const QString source = clip.value(QStringLiteral("sourceId")).toString();
    if (source.isEmpty()) return {};
    return sourceNames(timeline.value(QStringLiteral("sources")).toArray()).value(source);
}

} // namespace

FinishTray::FinishTray(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("finishTray"));
    m_analyser = new ScopeAnalyser(this);
    m_mode = QSettings().value(kModeKey).toString() == QStringLiteral("rgb") ? WaveformMode::Rgb : WaveformMode::Luma;

    // ---- the bar ----------------------------------------------------------
    m_bar = new QWidget(this);
    m_bar->setObjectName(QStringLiteral("finishBar"));
    auto *bar = new QHBoxLayout(m_bar);
    bar->setContentsMargins(tokens::space::panelLeft, 0, tokens::space::panelHeaderRight, 0);
    bar->setSpacing(tokens::space::s4);
    bar->addWidget(text(QStringLiteral("Scopes"), tokens::controlStrong, tokens::ink));
    bar->addSpacing(tokens::space::s4);
    m_scopes = text(QString(), tokens::meta, tokens::ink3);
    m_scopes->setObjectName(QStringLiteral("finishScopesMeta"));
    bar->addWidget(m_scopes);
    bar->addWidget(new ui::Separator(tokens::space::s4));
    m_ask = new ui::GhostButton(QStringLiteral("Ask"));
    m_ask->setObjectName(QStringLiteral("finishAsk"));
    m_ask->setIconName(QStringLiteral("g-ask"));
    connect(m_ask, &QAbstractButton::clicked, this, [this] {
        if (m_bus) m_bus->run(QStringLiteral("ai.ask"));
    });
    bar->addWidget(m_ask);
    bar->addStretch(1);
    m_targets = text(QString(), tokens::meta, tokens::ink3);
    m_targets->setObjectName(QStringLiteral("finishTargets"));
    bar->addWidget(m_targets);
    bar->addSpacing(tokens::space::s8);
    m_runQc = new ui::Button(QStringLiteral("Run QC"), ui::Button::Kind::Secondary);
    m_runQc->setObjectName(QStringLiteral("finishRunQc"));
    connect(m_runQc, &QAbstractButton::clicked, this, [this] {
        if (m_bus) m_bus->run(QStringLiteral("qc.run"));
    });
    bar->addWidget(m_runQc);

    // ---- the panes --------------------------------------------------------
    m_loudness = new LoudnessView(nullptr, bus);
    m_qc = new QcView(nullptr, bus);
    m_waveMeta = new ClippedMeta;
    shell::setTextStyle(m_waveMeta, tokens::meta, tokens::ink3);
    m_waveMeta->setObjectName(QStringLiteral("finishWaveformMeta"));
    m_vectorMeta = text(QStringLiteral("75%"), tokens::meta, tokens::ink3);
    m_vectorMeta->setObjectName(QStringLiteral("finishVectorscopeMeta"));
    m_paradeMeta = new ClippedMeta;
    shell::setTextStyle(m_paradeMeta, tokens::meta, tokens::ink3);
    m_paradeMeta->setObjectName(QStringLiteral("finishParadeMeta"));
    m_qcMeta = text(QString(), tokens::meta, tokens::ink3);
    m_qcMeta->setObjectName(QStringLiteral("finishQcMeta"));
    m_qcStandards = new ClippedMeta;
    shell::setTextStyle(m_qcStandards, tokens::meta, tokens::ink3);
    m_qcStandards->setObjectName(QStringLiteral("finishQcStandards"));
    const struct {
        const char *id;
        const char *name;
        QLabel *meta;
        QWidget *body;
    } specs[] = {
        {"waveform", "Waveform", m_waveMeta, new ScopePlot(ScopePlot::Kind::Waveform, m_analyser)},
        {"vectorscope", "Vectorscope", m_vectorMeta, new ScopePlot(ScopePlot::Kind::Vectorscope, m_analyser)},
        {"parade", "RGB parade", m_paradeMeta, new ScopePlot(ScopePlot::Kind::Parade, m_analyser)},
        {"loudness", "Loudness", text(QStringLiteral("Programme"), tokens::meta, tokens::ink3), m_loudness},
        {"qc", "QC", m_qcMeta, m_qc},
    };
    for (const auto &spec : specs) {
        const QString id = QString::fromLatin1(spec.id);
        auto *pane = new TrayPane(QStringLiteral("finishPane_%1").arg(id), QString::fromUtf8(spec.name), spec.meta, spec.body, this);
        m_panes.insert(id, pane);
        m_order << id;
        if (auto *plot = qobject_cast<ScopePlot *>(spec.body)) {
            m_plots.insert(id, plot);
            connect(plot, &ScopePlot::plotResized, this, &FinishTray::pushLayout);
        }
    }
    m_waveformMode = new ui::GhostButton;
    m_waveformMode->setObjectName(QStringLiteral("finishWaveformMode"));
    m_waveformMode->setTypeStyle(tokens::meta);
    m_waveformMode->setControlHeight(tokens::height::h20);
    m_waveformMode->setChevron(true);
    m_waveformMode->setTip(QStringLiteral("What the waveform traces"));
    connect(m_waveformMode, &QAbstractButton::clicked, this, &FinishTray::showWaveformMenu);
    static_cast<TrayPane *>(m_panes.value(QStringLiteral("waveform")))->addTrailing(m_waveformMode);
    static_cast<TrayPane *>(m_panes.value(QStringLiteral("qc")))->addTrailing(m_qcStandards);

    m_highlightTimer = new QTimer(this);
    m_highlightTimer->setSingleShot(true);
    connect(m_highlightTimer, &QTimer::timeout, this, [this] {
        m_highlighted.clear();
        for (QWidget *pane : std::as_const(m_panes)) static_cast<TrayPane *>(pane)->setHighlighted(false);
    });

    connect(m_analyser, &ScopeAnalyser::tracesChanged, this, &FinishTray::updateScopeMeta);
    if (m_bus) {
        connect(m_bus, &EditorBus::highlightPane, this, &FinishTray::highlightPane);
        connect(m_bus, &EditorBus::playheadChanged, this, &FinishTray::updateBar);
        connect(m_bus, &EditorBus::snapshotChanged, this, &FinishTray::updateBar);
        connect(m_bus, &EditorBus::programFrameChanged, m_analyser, &ScopeAnalyser::setFrame);
        connect(m_bus, &EditorBus::programFrameChanged, this, &FinishTray::askForSignal);
        connect(m_bus, &EditorBus::programSignalChanged, m_analyser, &ScopeAnalyser::setSignal);
        connect(m_bus, &EditorBus::event, this, [this](const QString &type, const QJsonObject &event) {
            if (type == QStringLiteral("media_info")) mergeMediaInfo(event.value(QStringLiteral("media")).toObject());
            if (type == QStringLiteral("scope_signal") && event.value(QStringLiteral("status")).toString() == QStringLiteral("sdr"))
                m_analyser->clearSignal();
            if (type == QStringLiteral("loudness") || type == QStringLiteral("check") || type == QStringLiteral("media_info") ||
                type == QStringLiteral("connection")) {
                // (QcView and LoudnessView, made first, have heard it already:
                // a connection ends whatever ran on the last one.)
                updateBar();
                if (type == QStringLiteral("check") || type == QStringLiteral("connection")) followActions();
            }
        });
        mergeMediaInfo(m_bus->lastEvent(QStringLiteral("media_info")).value(QStringLiteral("media")).toObject());
        m_analyser->setFrame(m_bus->programFrame());
    }
    applyWaveformMode(m_mode);
    updateBar();
    updateScopeMeta();
    followActions();
    resize(sizeHint());
}

QSize FinishTray::sizeHint() const {
    return QSize(tokens::layout::canvasWidth, tokens::layout::band);
}

QSize FinishTray::minimumSizeHint() const {
    return QSize(2 * kFixedWidth + 6 * kGap + 3 * 48, tokens::layout::strip + 2 * kGap + 120);
}

std::array<FinishTray::Span, 5> FinishTray::paneSpans(int width) {
    const int rest = std::max(0, width - 6 * kGap - 2 * kFixedWidth);
    const int shared = int(std::lround(double(rest) * kShare[2] / kShareTotal));
    const int qc = std::max(shared, std::min(kRoomyQc, rest - 2 * kLeastScope));
    const int waveform = (rest - qc) / 2;
    const int parade = rest - qc - waveform;
    const int widths[5] = {waveform, kFixedWidth, parade, kFixedWidth, qc};
    std::array<Span, 5> spans{};
    int x = kGap;
    for (int i = 0; i < 5; ++i) {
        spans[size_t(i)] = {x, widths[i]};
        x += widths[i] + kGap;
    }
    return spans;
}

void FinishTray::placePanes() {
    m_bar->setGeometry(0, 0, width(), tokens::layout::strip);
    const int top = tokens::layout::strip + kGap;
    const int height = std::max(0, this->height() - top - kGap);
    const auto spans = paneSpans(width());
    for (int i = 0; i < m_order.size(); ++i)
        m_panes.value(m_order.at(i))->setGeometry(spans[size_t(i)].x, top, spans[size_t(i)].width, height);
}

void FinishTray::resizeEvent(QResizeEvent *) {
    placePanes();
}

void FinishTray::showEvent(QShowEvent *) {
    placePanes();
    pushLayout();
    if (m_bus && !m_bus->programFrame().isNull() && m_analyser->traces().isNull()) m_analyser->setFrame(m_bus->programFrame());
    m_analyser->setActive(true);
    askForSignal();
}

void FinishTray::askForSignal() {
    // Only while the scopes are in sight: an HDR sequence draws its frame again for them.
    if (!m_bus || !m_analyser->isActive()) return;
    m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("scope_signal")}, {QStringLiteral("time"), m_bus->playhead()}});
}

void FinishTray::hideEvent(QHideEvent *) {
    m_analyser->setActive(false);
}

void FinishTray::pushLayout() {
    const ScopePlot *waveform = m_plots.value(QStringLiteral("waveform"));
    const ScopePlot *vectorscope = m_plots.value(QStringLiteral("vectorscope"));
    const ScopePlot *parade = m_plots.value(QStringLiteral("parade"));
    if (!waveform || !vectorscope || !parade) return;
    ScopeLayout layout;
    layout.waveform = waveform->plotSize();
    layout.vectorscope = vectorscope->plotSize();
    layout.parade = parade->plotSize();
    layout.dpr = devicePixelRatioF();
    layout.mode = m_mode;
    m_analyser->setLayout(layout);
}

void FinishTray::setWaveformMode(WaveformMode mode) {
    QSettings().setValue(kModeKey, mode == WaveformMode::Luma ? QStringLiteral("luma") : QStringLiteral("rgb"));
    applyWaveformMode(mode);
}

void FinishTray::applyWaveformMode(WaveformMode mode) {
    m_mode = mode;
    m_waveformMode->setText(mode == WaveformMode::Luma ? QStringLiteral("Luma") : QStringLiteral("RGB"));
    m_waveformMode->setFixedSize(m_waveformMode->sizeHint());
    pushLayout();
    updateScopeMeta();
}

void FinishTray::showWaveformMenu() {
    auto *menu = new QMenu(this);
    menu->setAttribute(Qt::WA_DeleteOnClose);
    ui::styleMenu(menu);
    const struct {
        const char *label;
        WaveformMode mode;
    } items[] = {{"Luma", WaveformMode::Luma}, {"RGB overlay", WaveformMode::Rgb}};
    for (const auto &item : items) {
        QAction *action = menu->addAction(QString::fromLatin1(item.label));
        action->setCheckable(true);
        action->setChecked(item.mode == m_mode);
        const WaveformMode mode = item.mode;
        connect(action, &QAction::triggered, this, [this, mode] { setWaveformMode(mode); });
    }
    ui::popupMenu(menu, m_waveformMode);
}

void FinishTray::setActions(QAction *ask, QAction *runQc) {
    for (QAction *old : {m_askAction.data(), m_runQcAction.data()})
        if (old) disconnect(old, nullptr, this, nullptr);
    m_askAction = ask;
    m_runQcAction = runQc;
    for (QAction *action : {ask, runQc})
        if (action) connect(action, &QAction::changed, this, &FinishTray::followActions);
    m_qc->setRunAction(runQc);
    followActions();
}

void FinishTray::followActions() {
    shell::followAction(m_ask, m_askAction);
    // One check runs at a time.
    shell::followAction(m_runQc, m_runQcAction, m_qc->running() ? QStringLiteral("QC is running") : QString());
}

void FinishTray::setLoudness(const QJsonObject &loudness) {
    m_loudness->setLoudness(loudness);
    updateBar();
}

void FinishTray::setCheck(const QJsonObject &check) {
    m_qc->setCheck(check);
    updateBar();
    followActions();
}

void FinishTray::highlightPane(const QString &id) {
    QSet<QString> ids;
    if (id == QStringLiteral("scopes")) ids = {QStringLiteral("waveform"), QStringLiteral("vectorscope"), QStringLiteral("parade")};
    else if (m_panes.contains(id)) ids = {id};
    if (ids.isEmpty()) return;
    m_highlighted = id;
    for (auto it = m_panes.cbegin(); it != m_panes.cend(); ++it)
        static_cast<TrayPane *>(it.value())->setHighlighted(ids.contains(it.key()));
    m_highlightTimer->start(highlightMs);
}

QString FinishTray::scopesText() const {
    return m_scopes->text();
}

QString FinishTray::targetsText() const {
    return m_targets->text();
}

QString FinishTray::paneMeta(const QString &id) const {
    if (id == QStringLiteral("waveform")) return m_waveMeta->text();
    if (id == QStringLiteral("vectorscope")) return m_vectorMeta->text();
    if (id == QStringLiteral("parade")) return m_paradeMeta->text();
    if (id == QStringLiteral("qc")) return m_qcMeta->text();
    if (id == QStringLiteral("loudness")) return QStringLiteral("Programme");
    return {};
}

void FinishTray::updateScopeMeta() {
    // The precision of the frame measured, as it is: frames come as RGBA8,
    // so 8-bit (never a claim of more), and an HDR sequence's as its signal.
    const int bits = m_analyser->traces().bitDepth;
    const int curve = m_analyser->traces().curve;
    const QString depth = curve == 1   ? QStringLiteral("PQ signal")
                          : curve == 2 ? QStringLiteral("HLG signal")
                          : bits > 0   ? QStringLiteral("%1-bit").arg(bits)
                                       : QString();
    const QString channel = m_mode == WaveformMode::Luma ? QStringLiteral("Y′") : QStringLiteral("R′G′B′");
    m_waveMeta->setText(depth.isEmpty() ? channel : QStringLiteral("%1 · %2").arg(channel, depth));
    m_paradeMeta->setText(depth);
}

void FinishTray::mergeMediaInfo(const QJsonObject &media) {
    for (auto it = media.constBegin(); it != media.constEnd(); ++it) m_mediaInfo.insert(it.key(), it.value().toObject());
}

void FinishTray::updateBar() {
    // "Program · 00:00:09:03 · B003_C011 · Rec.709": what the scopes measure.
    QStringList parts{QStringLiteral("Program")};
    if (m_bus) {
        parts << timelineTimecode(m_bus->playhead(), m_bus->fps());
        const QJsonObject timeline = m_bus->timeline();
        const QJsonObject clip = programClipAt(timeline, m_bus->playhead());
        if (!clip.isEmpty()) {
            if (const QString name = programClipName(timeline, clip); !name.isEmpty()) parts << name;
            const QString source = clip.value(QStringLiteral("sourceId")).toString();
            const QString colour = source.isEmpty() ? QString() : colourName(m_mediaInfo.value(source));
            if (!colour.isEmpty()) parts << colour;
        }
    }
    m_scopes->setText(parts.join(QStringLiteral(" · ")));
    // The R128 targets the measurement reports, else those QC checks against.
    const QJsonObject loudness = m_loudness->loudness();
    m_targets->setText(QStringLiteral("Loudness EBU R128 · %1 LUFS · %2 dBTP")
                           .arg(signedNumber(loudness.value(QStringLiteral("target")).toDouble(-23)),
                                signedNumber(loudness.value(QStringLiteral("limit")).toDouble(-1))));
    m_qcMeta->setText(m_qc->summaryText());
    m_qcStandards->setText(m_qc->standardsText());
}

void FinishTray::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), tokens::color(tokens::chrome));
    painter.fillRect(QRect(0, 0, width(), 1), tokens::color(tokens::line));
    painter.fillRect(QRect(0, tokens::layout::strip - 1, width(), 1), tokens::color(tokens::line));
}

} // namespace kerf
