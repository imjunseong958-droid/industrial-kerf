#include "effect_controls_panel.h"
#include "editor_common.h"

#include <QCheckBox>
#include <QColorDialog>
#include <QComboBox>
#include <QDoubleSpinBox>
#include <QFormLayout>
#include <QLabel>
#include <QLineEdit>
#include <QPixmap>
#include <QPushButton>
#include <QScrollArea>
#include <QSignalBlocker>
#include <QVBoxLayout>

#include <algorithm>
#include <cmath>

namespace cut {

EffectControlsPanel::EffectControlsPanel(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("effectControls"));
    setAttribute(Qt::WA_StyledBackground);
    auto *outer = new QVBoxLayout(this);
    outer->setContentsMargins(0, 0, 0, 0);
    auto *scroll = new QScrollArea;
    scroll->setFrameShape(QFrame::NoFrame);
    scroll->setWidgetResizable(true);
    scroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    outer->addWidget(scroll);
    auto *content = new QWidget;
    scroll->setWidget(content);
    auto *layout = new QVBoxLayout(content);
    layout->setContentsMargins(10, 8, 10, 8);
    layout->setSpacing(6);
    auto *timing = new QWidget;
    auto *form = new QFormLayout(timing);
    form->setContentsMargins(0, 0, 0, 0);
    form->setSpacing(6);
    m_clipName = new QLabel(QStringLiteral("Select a clip"));
    m_clipName->setObjectName(QStringLiteral("inspectorClip"));
    form->addRow(m_clipName);
    m_position = timingSpin(0.0, 86400.0, 0.01, QStringLiteral("timelineStart"));
    m_duration = timingSpin(0.01, 86400.0, 0.01, QStringLiteral("duration"));
    m_in = timingSpin(0.0, 86400.0, 0.01, QStringLiteral("in"));
    m_out = timingSpin(0.01, 86400.0, 0.01, QStringLiteral("out"));
    m_speed = timingSpin(0.01, 20.0, 0.01, QStringLiteral("speed"));
    form->addRow(QStringLiteral("Position"), m_position);
    form->addRow(QStringLiteral("Duration"), m_duration);
    form->addRow(QStringLiteral("Source in"), m_in);
    form->addRow(QStringLiteral("Source out"), m_out);
    form->addRow(QStringLiteral("Speed"), m_speed);
    layout->addWidget(timing);
    auto *properties = new QWidget;
    m_propertyForm = new QFormLayout(properties);
    m_propertyForm->setContentsMargins(0, 0, 0, 0);
    m_propertyForm->setSpacing(6);
    layout->addWidget(properties);
    layout->addStretch(1);
    setClip(QString(), QJsonObject{}, QJsonArray{}, m_fps);
}

QDoubleSpinBox *EffectControlsPanel::timingSpin(double minimum, double maximum, double step, const QString &field) {
    auto *spin = new QDoubleSpinBox;
    spin->setRange(minimum, maximum);
    spin->setSingleStep(step);
    spin->setDecimals(2);
    spin->setProperty("clipField", field);
    connect(spin, &QDoubleSpinBox::editingFinished, this, &EffectControlsPanel::timingEdited);
    return spin;
}

void EffectControlsPanel::setClip(const QString &id, const QJsonObject &clip, const QJsonArray &defs, double fps) {
    m_updating = true;
    m_clipId = id;
    m_clip = clip;
    m_fps = fps;
    const bool hasClip = !id.isEmpty();
    m_clipName->setText(hasClip ? clipName(clip) : QStringLiteral("Select a clip"));
    auto setSpin = [hasClip](QDoubleSpinBox *spin, double value) {
        spin->setEnabled(hasClip);
        const QSignalBlocker blocker(spin);
        spin->setValue(value);
    };
    setSpin(m_position, clip.value(QStringLiteral("timelineStart")).toDouble());
    setSpin(m_duration, std::max(0.01, clip.value(QStringLiteral("duration")).toDouble(1.0)));
    setSpin(m_in, clip.value(QStringLiteral("in")).toDouble());
    setSpin(m_out, std::max(0.01, clip.value(QStringLiteral("out")).toDouble(1.0)));
    setSpin(m_speed, clip.value(QStringLiteral("speed")).toDouble(1.0));
    // Property editors follow the catalog definitions; animated properties
    // show their first key and are read-only.
    if (defs != m_propertyDefs) {
        while (m_propertyForm->rowCount() > 0) m_propertyForm->removeRow(0);
        m_propertyEditors.clear();
        m_propertyDefs = defs;
        for (const QJsonValue &def : defs) {
            QWidget *editor = propertyEditor(def.toObject());
            const QString name = def.toObject().value(QStringLiteral("name")).toString();
            m_propertyEditors.insert(name, editor);
            m_propertyForm->addRow(name, editor);
        }
    }
    const QJsonObject props = clip.value(QStringLiteral("props")).toObject();
    for (const QJsonValue &def : defs) {
        const QString name = def.toObject().value(QStringLiteral("name")).toString();
        const QJsonValue prop = props.value(name);
        QWidget *editor = m_propertyEditors.value(name);
        setPropertyValue(editor, prop.isArray() ? prop.toArray().first().toObject().value(QStringLiteral("value"))
                                 : prop.isUndefined() ? def.toObject().value(QStringLiteral("default")) : prop);
        editor->setEnabled(!prop.isArray());
    }
    m_updating = false;
}

void EffectControlsPanel::edit(const QJsonObject &op) {
    if (!op.isEmpty()) emit editRequested(QJsonArray{op});
}

void EffectControlsPanel::timingEdited() {
    if (m_clipId.isEmpty() || m_updating) return;
    const QString field = sender()->property("clipField").toString();
    const auto *spin = qobject_cast<QDoubleSpinBox *>(sender());
    if (!spin || field.isEmpty()) return;
    const double value = spin->value();
    const double speed = std::max(0.01, m_clip.value(QStringLiteral("speed")).toDouble(1.0));
    const auto frames = [this](const QString &key) { return static_cast<qint64>(m_clip.value(key).toDouble()); };
    const auto frameAt = [this](double seconds) { return std::llround(seconds * m_fps); };
    auto amount = [this](const QString &op, double count, const QString &edge = QString()) {
        const qint64 whole = std::llround(count);
        if (whole == 0) return QJsonObject{};
        QJsonObject edit{{QStringLiteral("op"), op}, {QStringLiteral("clip"), m_clipId},
                         {QStringLiteral("frames"), whole}};
        if (!edge.isEmpty()) edit.insert(QStringLiteral("edge"), edge);
        return edit;
    };
    if (field == QStringLiteral("timelineStart")) {
        edit(amount(QStringLiteral("move"), double(frameAt(value) - frames(QStringLiteral("startFrame")))));
    } else if (field == QStringLiteral("duration")) {
        edit(amount(QStringLiteral("trim"), double(frameAt(value) - frames(QStringLiteral("durationFrames"))), QStringLiteral("out")));
    } else if (field == QStringLiteral("in")) {
        edit(amount(QStringLiteral("slip"), (value - m_clip.value(QStringLiteral("in")).toDouble()) / speed * m_fps));
    } else if (field == QStringLiteral("out")) {
        edit(amount(QStringLiteral("trim"), (value - m_clip.value(QStringLiteral("out")).toDouble()) / speed * m_fps, QStringLiteral("out")));
    } else if (field == QStringLiteral("speed")) {
        QString ratio = QString::number(value, 'f', 4);
        while (ratio.endsWith(QLatin1Char('0'))) ratio.chop(1);
        if (ratio.endsWith(QLatin1Char('.'))) ratio.chop(1);
        if (m_clip.value(QStringLiteral("reverse")).toBool()) ratio.prepend(QLatin1Char('-'));
        edit(QJsonObject{{QStringLiteral("op"), QStringLiteral("speed")}, {QStringLiteral("clip"), m_clipId},
                         {QStringLiteral("speed"), ratio}});
    }
}

void EffectControlsPanel::propertyEdited(const QString &name, const QJsonValue &value) {
    if (m_clipId.isEmpty() || m_updating) return;
    QJsonValue fallback;
    for (const QJsonValue &def : m_propertyDefs)
        if (def.toObject().value(QStringLiteral("name")).toString() == name) fallback = def.toObject().value(QStringLiteral("default"));
    const QJsonValue current = m_clip.value(QStringLiteral("props")).toObject().value(name);
    auto same = [](const QJsonValue &a, const QJsonValue &b) {
        return a.isDouble() && b.isDouble() ? std::abs(a.toDouble() - b.toDouble()) < 5e-4 : a == b;
    };
    if (same(value, current.isUndefined() ? fallback : current)) return;
    edit(QJsonObject{{QStringLiteral("op"), QStringLiteral("set")}, {QStringLiteral("clip"), m_clipId},
                     {QStringLiteral("props"), QJsonObject{{name, same(value, fallback) ? QJsonValue() : value}}}});
}

QWidget *EffectControlsPanel::propertyEditor(const QJsonObject &def) {
    const QString name = def.value(QStringLiteral("name")).toString();
    const QString type = def.value(QStringLiteral("type")).toString();
    QWidget *editor = nullptr;
    if (type == QStringLiteral("number")) {
        auto *spin = new QDoubleSpinBox;
        const double minimum = def.value(QStringLiteral("min")).toDouble(-1e6);
        const double maximum = def.value(QStringLiteral("max")).toDouble(1e6);
        spin->setRange(minimum, maximum);
        spin->setDecimals(3);
        spin->setSingleStep(maximum - minimum > 10.0 ? 1.0 : 0.01);
        connect(spin, &QDoubleSpinBox::editingFinished, this, [this, spin, name] { propertyEdited(name, spin->value()); });
        editor = spin;
    } else if (type == QStringLiteral("enum")) {
        auto *combo = new QComboBox;
        for (const QJsonValue &option : def.value(QStringLiteral("options")).toArray()) combo->addItem(option.toString());
        connect(combo, &QComboBox::activated, this, [this, combo, name] { propertyEdited(name, combo->currentText()); });
        editor = combo;
    } else if (type == QStringLiteral("bool")) {
        auto *check = new QCheckBox;
        connect(check, &QCheckBox::clicked, this, [this, name](bool checked) { propertyEdited(name, checked); });
        editor = check;
    } else if (type == QStringLiteral("color")) {
        auto *button = new QPushButton;
        connect(button, &QPushButton::clicked, this, [this, button, name] {
            const QColor color = QColorDialog::getColor(modelColor(button->text()), this, name,
                                                        QColorDialog::ShowAlphaChannel);
            if (color.isValid()) propertyEdited(name, modelColorName(color));
        });
        editor = button;
    } else {
        auto *line = new QLineEdit;
        connect(line, &QLineEdit::editingFinished, this, [this, line, name] { propertyEdited(name, line->text()); });
        editor = line;
    }
    QString tip = def.value(QStringLiteral("doc")).toString();
    if (def.value(QStringLiteral("unit")).isString()) tip += QStringLiteral(" (%1)").arg(def.value(QStringLiteral("unit")).toString());
    editor->setToolTip(tip.trimmed());
    return editor;
}

void EffectControlsPanel::setPropertyValue(QWidget *editor, const QJsonValue &value) {
    const QSignalBlocker blocker(editor);
    if (auto *spin = qobject_cast<QDoubleSpinBox *>(editor)) spin->setValue(value.toDouble());
    else if (auto *combo = qobject_cast<QComboBox *>(editor)) combo->setCurrentText(value.toString());
    else if (auto *check = qobject_cast<QCheckBox *>(editor)) check->setChecked(value.toBool());
    else if (auto *button = qobject_cast<QPushButton *>(editor)) {
        QPixmap swatch(12, 12);
        swatch.fill(modelColor(value.toString()));
        button->setIcon(swatch);
        button->setText(value.toString());
    }
    else if (auto *line = qobject_cast<QLineEdit *>(editor)) line->setText(value.toString());
}

} // namespace cut
