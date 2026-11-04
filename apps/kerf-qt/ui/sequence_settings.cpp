#include "sequence_settings.h"

#include "components.h"
#include "design_tokens.h"
#include "editor_common.h"
#include "inspector_presentation.h"
#include "toolbar_widgets.h"
#include "tray_shell.h"

#include <QComboBox>
#include <QGridLayout>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QLabel>
#include <QLineEdit>
#include <QPainter>
#include <QRegularExpression>
#include <QRegularExpressionValidator>
#include <QSpinBox>
#include <QVBoxLayout>

#include <algorithm>
#include <cmath>
#include <numeric>
#include <optional>
#include <utility>

namespace kerf {

namespace {

using tokens::color;

constexpr int kPad = 20;
constexpr int kLabelWidth = 96;
constexpr int kMinSide = 16;
constexpr int kMaxSide = 8192;

// shell::text's ink and size, kept under the app's sheet (its `*` rule
// gives every widget the text colour and 12 px).
void setInk(QLabel *label, const tokens::TypeStyle &style, QRgb ink) {
    label->setStyleSheet(QStringLiteral("QLabel { color: %1; font-size: %2px; font-weight: %3; background: transparent; }")
                             .arg(tokens::color(ink).name(QColor::HexRgb))
                             .arg(style.px)
                             .arg(style.weight));
}

QLabel *inked(const QString &content, const tokens::TypeStyle &style, QRgb ink) {
    QLabel *label = shell::text(content, style, ink);
    setInk(label, style, ink);
    return label;
}

QLabel *rowLabel(const QString &text) {
    QLabel *label = inked(text, tokens::control, tokens::ink2);
    label->setFixedWidth(kLabelWidth);
    return label;
}

// The preset a size is, or -1 for a size of one's own.
int presetOf(const QSize &size) {
    const QList<CanvasPreset> presets = canvasPresets();
    for (int i = 0; i < presets.size(); ++i)
        if (size == QSize(presets.at(i).width, presets.at(i).height)) return i;
    return -1;
}

} // namespace

QString SequenceSettingsDialog::rateNote() {
    return QStringLiteral("Frame rate can change only while the sequence is empty");
}

QList<QPair<QString, QString>> SequenceSettingsDialog::rates() {
    // kerfd's own spellings (Rate's Display), so a snapshot's rate reads
    // back as the same row.
    return {{QStringLiteral("23.976p"), QStringLiteral("23.976")}, {QStringLiteral("24p"), QStringLiteral("24")},
            {QStringLiteral("25p"), QStringLiteral("25")},         {QStringLiteral("29.97p"), QStringLiteral("29.97")},
            {QStringLiteral("30p"), QStringLiteral("30")},         {QStringLiteral("50p"), QStringLiteral("50")},
            {QStringLiteral("59.94p"), QStringLiteral("59.94")},   {QStringLiteral("60p"), QStringLiteral("60")}};
}

std::optional<QPair<qint64, qint64>> SequenceSettingsDialog::rateFraction(const QString &rate) {
    // Rate::parse's reading: "num/den", a whole number, or a decimal that is
    // an NTSC rate (within 0.005) or else exactly what it says.
    const QString text = rate.trimmed();
    const auto reduced = [](qint64 num, qint64 den) -> std::optional<QPair<qint64, qint64>> {
        if (num <= 0 || den <= 0 || num > 0xffffffffLL || den > 0xffffffffLL) return std::nullopt;
        const qint64 divisor = std::gcd(num, den);
        return QPair<qint64, qint64>(num / divisor, den / divisor);
    };
    const auto digitsOnly = [](const QString &part) {
        return std::all_of(part.cbegin(), part.cend(), [](QChar c) { return c >= QLatin1Char('0') && c <= QLatin1Char('9'); });
    };
    if (const qsizetype slash = text.indexOf(QLatin1Char('/')); slash >= 0) {
        const QString num = text.left(slash);
        const QString den = text.mid(slash + 1);
        if (num.isEmpty() || den.isEmpty() || !digitsOnly(num) || !digitsOnly(den)) return std::nullopt;
        return reduced(num.toLongLong(), den.toLongLong());
    }
    const qsizetype dot = text.indexOf(QLatin1Char('.'));
    const QString whole = dot < 0 ? text : text.left(dot);
    const QString fraction = dot < 0 ? QString() : text.mid(dot + 1);
    if (whole.isEmpty() || whole.size() > 9 || fraction.size() > 6 || !digitsOnly(whole) || !digitsOnly(fraction)) return std::nullopt;
    qint64 scale = 1;
    for (qsizetype i = 0; i < fraction.size(); ++i) scale *= 10;
    const qint64 digits = (whole + fraction).toLongLong();
    if (digits % scale == 0) return reduced(digits / scale, 1);
    const double value = double(digits) / double(scale);
    const double nominal = std::round(value * 1.001);
    if (nominal >= 1 && std::abs(value - nominal * 1000.0 / 1001.0) < 0.005) return reduced(qint64(nominal) * 1000, 1001);
    return reduced(digits, scale);
}

bool SequenceSettingsDialog::sameRate(const QString &a, const QString &b) {
    const auto left = rateFraction(a);
    const auto right = rateFraction(b);
    return left && right ? *left == *right : a.trimmed() == b.trimmed();
}

QJsonObject SequenceSettingsDialog::rateValue(const QString &rate) {
    const auto fraction = rateFraction(rate);
    if (!fraction) return {};
    return QJsonObject{{QStringLiteral("num"), fraction->first}, {QStringLiteral("den"), fraction->second}};
}

int SequenceSettingsDialog::findRate(const QString &rate) const {
    for (int i = 0; i < m_rate->count(); ++i)
        if (sameRate(m_rate->itemData(i).toString(), rate)) return i;
    return -1;
}

void SequenceSettingsDialog::showRate(const Sequence &sequence) {
    int index = findRate(sequence.rate);
    if (index < 0) {
        m_rate->addItem(frameRateLabel(sequence.fps), sequence.rate);
        index = m_rate->count() - 1;
    }
    m_rate->setCurrentIndex(index);
}

SequenceSettingsDialog::Sequence SequenceSettingsDialog::fromTimeline(const QJsonObject &timeline) {
    Sequence sequence;
    sequence.id = timeline.value(QStringLiteral("sequence")).toString();
    sequence.name = timeline.value(QStringLiteral("name")).toString();
    sequence.size = QSize(timeline.value(QStringLiteral("width")).toInt(), timeline.value(QStringLiteral("height")).toInt());
    sequence.rate = timeline.value(QStringLiteral("rate")).toString();
    sequence.fps = timeline.value(QStringLiteral("fps")).toDouble();
    sequence.dropFrame = timeline.value(QStringLiteral("dropFrame")).toBool();
    sequence.start = timeline.value(QStringLiteral("start")).toDouble();
    // The program sequence's own entry names it and says its format.
    for (const QJsonValue &value : timeline.value(QStringLiteral("sequences")).toArray()) {
        const QJsonObject entry = value.toObject();
        if (entry.value(QStringLiteral("id")).toString() != sequence.id) continue;
        if (entry.contains(QStringLiteral("name"))) sequence.name = entry.value(QStringLiteral("name")).toString();
        if (entry.value(QStringLiteral("width")).toInt() > 0)
            sequence.size = QSize(entry.value(QStringLiteral("width")).toInt(), entry.value(QStringLiteral("height")).toInt());
        if (!entry.value(QStringLiteral("rate")).toString().isEmpty()) sequence.rate = entry.value(QStringLiteral("rate")).toString();
        if (entry.value(QStringLiteral("fps")).toDouble() > 0) sequence.fps = entry.value(QStringLiteral("fps")).toDouble();
        if (entry.contains(QStringLiteral("dropFrame"))) sequence.dropFrame = entry.value(QStringLiteral("dropFrame")).toBool();
    }
    if (sequence.fps <= 0) sequence.fps = 24.0;
    if (sequence.rate.isEmpty()) {
        const double whole = std::round(sequence.fps);
        // Written as kerfd writes one: "25", "29.97".
        sequence.rate = std::abs(sequence.fps - whole) < 0.001 ? QString::number(qint64(whole)) : QString::number(sequence.fps, 'f', 3);
        while (sequence.rate.endsWith(QLatin1Char('0'))) sequence.rate.chop(1);
    }
    bool empty = timeline.value(QStringLiteral("markers")).toArray().isEmpty();
    for (const QJsonValue &value : timeline.value(QStringLiteral("tracks")).toArray())
        empty = empty && value.toObject().value(QStringLiteral("clips")).toArray().isEmpty();
    sequence.empty = empty;
    return sequence;
}

QString SequenceSettingsDialog::startText(const Sequence &sequence) {
    return frameTimecode(std::llround(sequence.start * sequence.fps), sequence.fps, sequence.dropFrame);
}

bool SequenceSettingsDialog::holds(const QJsonObject &timeline, const QJsonObject &op) {
    if (op.isEmpty()) return false;
    const Sequence sequence = fromTimeline(timeline);
    if (op.contains(QStringLiteral("sequence")) && op.value(QStringLiteral("sequence")).toString() != sequence.id) return false;
    if (op.contains(QStringLiteral("name")) && op.value(QStringLiteral("name")).toString() != sequence.name) return false;
    if (op.contains(QStringLiteral("width")) &&
        QSize(op.value(QStringLiteral("width")).toInt(), op.value(QStringLiteral("height")).toInt()) != sequence.size)
        return false;
    if (op.contains(QStringLiteral("rate")) && rateValue(sequence.rate) != op.value(QStringLiteral("rate")).toObject()) return false;
    if (op.contains(QStringLiteral("start"))) {
        const QString start = op.value(QStringLiteral("start")).toString();
        if (startText(sequence) != start) {
            const bool drop = start.contains(QLatin1Char(';'));
            bool relative = false;
            const std::optional<qint64> frames = parseTimecode(start, sequence.fps, drop, &relative);
            if (!frames || relative || *frames != std::llround(sequence.start * sequence.fps) || sequence.dropFrame != drop) return false;
        }
    }
    return true;
}

SequenceSettingsDialog::SequenceSettingsDialog(const QJsonObject &timeline, QWidget *parent) : QDialog(parent) {
    setObjectName(QStringLiteral("sequenceSettingsDialog"));
    setWindowTitle(QStringLiteral("Sequence Settings"));
    setFixedWidth(kWidth);
    m_sequence = fromTimeline(timeline);

    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(kPad, kPad, kPad, kPad);
    layout->setSpacing(tokens::space::s12);
    auto *header = new QHBoxLayout;
    header->setSpacing(tokens::space::s8);
    QLabel *title = inked(QStringLiteral("Sequence settings"), tokens::title, tokens::ink);
    title->setObjectName(QStringLiteral("sequenceSettingsTitle"));
    header->addWidget(title);
    QLabel *meta = inked(m_sequence.id, tokens::meta, tokens::ink3);
    meta->setObjectName(QStringLiteral("sequenceSettingsId"));
    header->addWidget(meta, 0, Qt::AlignBaseline);
    header->addStretch(1);
    layout->addLayout(header);

    auto *grid = new QGridLayout;
    grid->setHorizontalSpacing(tokens::space::s12);
    grid->setVerticalSpacing(tokens::space::s8);
    int row = 0;

    // Name.
    m_name = new QLineEdit(m_sequence.name);
    m_name->setObjectName(QStringLiteral("sequenceName"));
    m_name->setFont(tokens::font(tokens::control));
    m_name->setAccessibleName(QStringLiteral("Sequence name"));
    grid->addWidget(rowLabel(QStringLiteral("Name")), row, 0);
    grid->addWidget(m_name, row++, 1);

    // Format: the four presets, or a size of one's own.
    m_format = new ui::SegmentedControl;
    m_format->setObjectName(QStringLiteral("sequenceFormat"));
    const QList<CanvasPreset> presets = canvasPresets();
    for (const CanvasPreset &preset : presets)
        m_format->addSegment(QString::fromLatin1(preset.ratio), QStringLiteral("%1 × %2").arg(preset.width).arg(preset.height));
    m_format->addSegment(QStringLiteral("Custom"), QStringLiteral("A frame size of your own"));
    grid->addWidget(rowLabel(QStringLiteral("Format")), row, 0);
    grid->addWidget(m_format, row++, 1, Qt::AlignLeft);
    m_custom = new QWidget;
    auto *custom = new QHBoxLayout(m_custom);
    custom->setContentsMargins(0, 0, 0, 0);
    custom->setSpacing(tokens::space::s8);
    const auto side = [](const QString &name) {
        auto *field = new QSpinBox;
        field->setObjectName(name);
        field->setRange(kMinSide, kMaxSide);
        field->setSingleStep(2);
        field->setButtonSymbols(QAbstractSpinBox::NoButtons);
        field->setFont(tokens::font(tokens::timecode));
        field->setFixedWidth(72);
        field->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
        return field;
    };
    m_widthField = side(QStringLiteral("sequenceWidth"));
    m_heightField = side(QStringLiteral("sequenceHeight"));
    m_widthField->setAccessibleName(QStringLiteral("Width"));
    m_heightField->setAccessibleName(QStringLiteral("Height"));
    m_widthField->setValue(std::clamp(m_sequence.size.width(), kMinSide, kMaxSide));
    m_heightField->setValue(std::clamp(m_sequence.size.height(), kMinSide, kMaxSide));
    custom->addWidget(m_widthField);
    custom->addWidget(inked(QStringLiteral("×"), tokens::control, tokens::ink3));
    custom->addWidget(m_heightField);
    custom->addStretch(1);
    grid->addWidget(m_custom, row++, 1);
    const int preset = presetOf(m_sequence.size);
    m_format->setCurrentIndex(preset >= 0 ? preset : int(presets.size()), false);
    connect(m_format, &ui::SegmentedControl::activated, this, &SequenceSettingsDialog::chooseFormat);

    // Start timecode.
    m_start = new QLineEdit(startText(m_sequence));
    m_start->setObjectName(QStringLiteral("sequenceStart"));
    m_start->setFont(tokens::font(tokens::timecode));
    m_start->setAccessibleName(QStringLiteral("Start timecode"));
    m_start->setValidator(new QRegularExpressionValidator(QRegularExpression(QStringLiteral("^\\d{0,2}(:\\d{0,2}(:\\d{0,2}([:;]\\d{0,2})?)?)?$")), m_start));
    m_start->setFixedWidth(120);
    grid->addWidget(rowLabel(QStringLiteral("Start")), row, 0);
    grid->addWidget(m_start, row++, 1, Qt::AlignLeft);

    // Frame rate.
    m_rate = new QComboBox;
    m_rate->setObjectName(QStringLiteral("sequenceRate"));
    m_rate->setFont(tokens::font(tokens::control));
    m_rate->setAccessibleName(QStringLiteral("Frame rate"));
    for (const auto &[label, rate] : rates()) m_rate->addItem(label, rate);
    showRate(m_sequence);
    m_rate->setFixedWidth(120);
    grid->addWidget(rowLabel(QStringLiteral("Frame rate")), row, 0);
    grid->addWidget(m_rate, row++, 1, Qt::AlignLeft);
    m_rateNote = inked(rateNote(), tokens::meta, tokens::ink3);
    m_rateNote->setObjectName(QStringLiteral("sequenceRateNote"));
    m_rateNote->setWordWrap(true);
    grid->addWidget(m_rateNote, row++, 1);
    grid->setColumnStretch(1, 1);
    layout->addLayout(grid);

    // kerfd's refusal, in neg.
    m_error = inked(QString(), tokens::meta, tokens::neg);
    m_error->setObjectName(QStringLiteral("sequenceSettingsError"));
    m_error->setWordWrap(true);
    m_error->hide();
    layout->addWidget(m_error);

    auto *buttons = new QHBoxLayout;
    buttons->setSpacing(tokens::space::s8);
    buttons->addStretch(1);
    m_cancel = new ui::Button(QStringLiteral("Cancel"), ui::Button::Kind::Secondary);
    m_cancel->setObjectName(QStringLiteral("sequenceSettingsCancel"));
    m_apply = new ui::Button(QStringLiteral("Apply"), ui::Button::Kind::Primary);
    m_apply->setObjectName(QStringLiteral("sequenceSettingsApply"));
    m_cancel->setTip(QStringLiteral("Close without changing the sequence"), QStringLiteral("Esc"));
    m_apply->setTip(QStringLiteral("Change the sequence"), QStringLiteral("↩"));
    buttons->addWidget(m_cancel);
    buttons->addWidget(m_apply);
    layout->addLayout(buttons);
    connect(m_cancel, &QAbstractButton::clicked, this, &QDialog::reject);
    connect(m_apply, &QAbstractButton::clicked, this, &SequenceSettingsDialog::apply);
    for (QLineEdit *field : {m_name, m_start}) {
        connect(field, &QLineEdit::textChanged, this, &SequenceSettingsDialog::syncApply);
        connect(field, &QLineEdit::returnPressed, this, &SequenceSettingsDialog::apply);
    }
    for (QSpinBox *field : {m_widthField, m_heightField}) connect(field, &QSpinBox::valueChanged, this, &SequenceSettingsDialog::syncApply);
    connect(m_rate, &QComboBox::currentIndexChanged, this, &SequenceSettingsDialog::syncApply);

    chooseFormat(m_format->currentIndex());
    syncRate();
    syncApply();
}

void SequenceSettingsDialog::setTimeline(const QJsonObject &timeline) {
    const Sequence fresh = fromTimeline(timeline);
    const Sequence before = std::exchange(m_sequence, fresh);
    // A field nobody has touched follows the sequence.
    if (m_name->text() == before.name) m_name->setText(fresh.name);
    if (m_start->text() == startText(before)) m_start->setText(startText(fresh));
    if (sameRate(m_rate->currentData().toString(), before.rate)) showRate(fresh);
    if (chosenSize() == before.size) {
        const int preset = presetOf(fresh.size);
        m_widthField->setValue(std::clamp(fresh.size.width(), kMinSide, kMaxSide));
        m_heightField->setValue(std::clamp(fresh.size.height(), kMinSide, kMaxSide));
        m_format->setCurrentIndex(preset >= 0 ? preset : int(canvasPresets().size()), false);
        chooseFormat(m_format->currentIndex());
    }
    syncRate();
    syncApply();
}

QSize SequenceSettingsDialog::chosenSize() const {
    const QList<CanvasPreset> presets = canvasPresets();
    const int index = m_format->currentIndex();
    if (index >= 0 && index < presets.size()) return QSize(presets.at(index).width, presets.at(index).height);
    return QSize(m_widthField->value(), m_heightField->value());
}

void SequenceSettingsDialog::chooseFormat(int index) {
    const QList<CanvasPreset> presets = canvasPresets();
    const bool custom = index < 0 || index >= presets.size();
    if (!custom) {
        m_widthField->setValue(presets.at(index).width);
        m_heightField->setValue(presets.at(index).height);
    }
    m_widthField->setEnabled(custom);
    m_heightField->setEnabled(custom);
    syncApply();
}

bool SequenceSettingsDialog::rateEditable() const {
    return m_sequence.empty;
}

void SequenceSettingsDialog::syncRate() {
    // With content the rate is the sequence's: it shows, read-only, and says why.
    m_rate->setEnabled(rateEditable());
    m_rateNote->setVisible(!rateEditable());
    if (!rateEditable()) {
        showRate(m_sequence);
    }
}

QJsonObject SequenceSettingsDialog::op(QString *problem) const {
    QJsonObject op{{QStringLiteral("op"), QStringLiteral("set-sequence")}};
    if (!m_sequence.id.isEmpty()) op.insert(QStringLiteral("sequence"), m_sequence.id);
    const QString name = m_name->text().trimmed();
    if (name.isEmpty()) {
        if (problem) *problem = QStringLiteral("A sequence needs a name");
        return {};
    }
    if (name != m_sequence.name) op.insert(QStringLiteral("name"), name);
    const QSize size = chosenSize();
    if (size != m_sequence.size) {
        op.insert(QStringLiteral("width"), size.width());
        op.insert(QStringLiteral("height"), size.height());
    }
    const QString rate = m_rate->currentData().toString();
    const bool rateChanged = rateEditable() && !sameRate(rate, m_sequence.rate);
    if (rateChanged) {
        const QJsonObject value = rateValue(rate);
        if (value.isEmpty()) {
            if (problem) *problem = QStringLiteral("Choose a frame rate");
            return {};
        }
        op.insert(QStringLiteral("rate"), value);
    }
    const QString start = m_start->text().trimmed();
    if (start != startText(m_sequence)) {
        const QJsonObject newRate = rateValue(rate);
        const double fps = rateChanged ? newRate.value(QStringLiteral("num")).toDouble() / std::max(1, newRate.value(QStringLiteral("den")).toInt())
                                       : m_sequence.fps;
        bool relative = false;
        const std::optional<qint64> frames = parseTimecode(start, fps, start.contains(QLatin1Char(';')), &relative);
        if (!frames || relative || *frames < 0 || start.count(QLatin1Char(':')) + start.count(QLatin1Char(';')) != 3) {
            if (problem) *problem = QStringLiteral("Write the start as hours:minutes:seconds:frames");
            return {};
        }
        op.insert(QStringLiteral("start"), start);
    }
    if (problem) problem->clear();
    // Only the sequence's id: nothing changed.
    return op.size() <= (m_sequence.id.isEmpty() ? 1 : 2) ? QJsonObject{} : op;
}

void SequenceSettingsDialog::syncApply() {
    QString problem;
    const QJsonObject change = op(&problem);
    m_apply->setEnabled(problem.isEmpty());
    m_apply->setTip(problem.isEmpty() ? (change.isEmpty() ? QStringLiteral("Nothing changed") : QStringLiteral("Change the sequence"))
                                      : problem,
                    QStringLiteral("↩"));
}

void SequenceSettingsDialog::apply() {
    QString problem;
    const QJsonObject change = op(&problem);
    if (!problem.isEmpty()) {
        setError(problem);
        return;
    }
    if (change.isEmpty()) {
        accept();
        return;
    }
    setError(QString());
    emit applyRequested(change);
}

void SequenceSettingsDialog::setError(const QString &message) {
    m_error->setText(message);
    m_error->setVisible(!message.isEmpty());
    adjustSize();
}

QString SequenceSettingsDialog::error() const {
    return m_error->text();
}

void SequenceSettingsDialog::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), color(tokens::card));
}

} // namespace kerf
